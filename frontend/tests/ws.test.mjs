import assert from 'node:assert/strict';
import { after, afterEach, before, beforeEach, test } from 'node:test';
import { createServer } from 'vite';

let server;
let subscribeProjectsWebSocket;
let subscribeProjectWebSocket;
let timers;
let sockets;
let cleanups;
let originalWindow;
let originalWebSocket;

before(async () => {
  server = await createServer({
    configFile: false,
    envDir: false,
    server: { middlewareMode: true, hmr: false, watch: null },
  });
  ({ subscribeProjectsWebSocket, subscribeProjectWebSocket } =
    await server.ssrLoadModule('/src/api/ws.ts'));
});

after(async () => { await server?.close(); });

beforeEach(() => {
  timers = new Map();
  sockets = [];
  cleanups = [];
  originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  originalWebSocket = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket');
  let timerId = 0;
  globalThis.window = {
    location: { protocol: 'https:', host: 'example.test' },
    setTimeout(callback, delay) {
      timers.set(++timerId, { callback, delay });
      return timerId;
    },
    clearTimeout(id) { timers.delete(id); },
  };
  globalThis.WebSocket = class {
    closeCalls = 0;
    constructor(url) {
      this.url = url;
      sockets.push(this);
    }
    close() {
      this.closeCalls++;
      this.onclose?.();
    }
    message(data) { this.onmessage?.({ data }); }
  };
});

afterEach(() => {
  for (const cleanup of cleanups) cleanup();
  for (const [key, descriptor] of [
    ['window', originalWindow], ['WebSocket', originalWebSocket],
  ]) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else delete globalThis[key];
  }
});

function subscribe(onUpdate = () => {}, onDisconnect = () => {}) {
  const cleanup = subscribeProjectsWebSocket(onUpdate, onDisconnect);
  cleanups.push(cleanup);
  return cleanup;
}

function retry(expectedDelay) {
  assert.equal(timers.size, 1);
  const [id, { callback, delay }] = [...timers][0];
  assert.equal(delay, expectedDelay);
  timers.delete(id);
  callback();
  return sockets.at(-1);
}

test('unsubscribe suppresses queued events and closes only once', () => {
  let updates = 0;
  let disconnects = 0;
  const cleanup = subscribe(() => updates++, () => disconnects++);
  const socket = sockets[0];
  const lateMessage = socket.onmessage;
  const lateClose = socket.onclose;
  const lateError = socket.onerror;
  cleanup();
  cleanup();
  lateMessage({ data: '{"type":"projects","data":[]}' });
  lateClose();
  lateError();
  assert.equal(updates, 0);
  assert.equal(disconnects, 0);
  assert.equal(socket.closeCalls, 1);
  assert.equal(timers.size, 0);
});

test('unsubscribe cancels the pending reconnect timer', () => {
  const cleanup = subscribe();
  sockets[0].onclose();
  assert.equal(timers.size, 1);
  cleanup();
  assert.equal(timers.size, 0);
  assert.equal(sockets.length, 1);
});

test('reconnect backs off to the cap and resets after opening', () => {
  let disconnects = 0;
  subscribe(undefined, () => disconnects++);
  let socket = sockets[0];
  for (const delay of [1000, 2000, 4000, 8000, 15000, 15000]) {
    socket.onclose();
    socket = retry(delay);
  }
  assert.equal(disconnects, 6);
  socket.onopen();
  socket.onclose();
  retry(1000);
});

test('obsolete socket events cannot update or close a new connection', () => {
  let updates = 0;
  let disconnects = 0;
  subscribe(() => updates++, () => disconnects++);
  const oldSocket = sockets[0];
  const lateMessage = oldSocket.onmessage;
  const lateClose = oldSocket.onclose;
  const lateError = oldSocket.onerror;
  oldSocket.onclose();
  const newSocket = retry(1000);
  lateMessage({ data: '{"type":"projects","data":[]}' });
  lateError();
  lateClose();
  assert.equal(updates, 0);
  assert.equal(disconnects, 1);
  assert.equal(newSocket.closeCalls, 0);
  assert.equal(timers.size, 0);
});

test('disconnect callback can unsubscribe without scheduling a reconnect', () => {
  let cleanup;
  cleanup = subscribe(undefined, () => cleanup());
  sockets[0].onclose();
  assert.equal(timers.size, 0);
});

test('subscriptions route their message type and ignore malformed messages', () => {
  const projectUpdates = [];
  const listUpdates = [];
  subscribe((value) => listUpdates.push(value));
  cleanups.push(subscribeProjectWebSocket('project-1', (value) => projectUpdates.push(value)));
  const [listSocket, projectSocket] = sockets;
  assert.equal(listSocket.url, 'wss://example.test/v1/projects/ws');
  assert.equal(projectSocket.url, 'wss://example.test/v1/projects/project-1/ws');
  for (const socket of sockets) {
    for (const raw of ['broken JSON', 'null', '{}', '{"type":"other"}']) socket.message(raw);
    socket.message('{"type":"project","data":{"id":"project-1"}}');
    socket.message('{"type":"projects","data":[{"id":"project-1"}]}');
  }
  assert.deepEqual(projectUpdates, [{ id: 'project-1' }]);
  assert.deepEqual(listUpdates, [[{ id: 'project-1' }]]);
});
