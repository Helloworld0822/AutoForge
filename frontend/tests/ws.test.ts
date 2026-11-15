import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { subscribeProjectsWebSocket } from '../src/api/ws';

class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  close = vi.fn(() => this.onclose?.());
  constructor(public url: string) { FakeSocket.instances.push(this); }
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSocket.instances = [];
  vi.stubGlobal('WebSocket', FakeSocket);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('detaches callbacks on cleanup and ignores queued messages', () => {
  const update = vi.fn();
  const disconnect = vi.fn();
  const stop = subscribeProjectsWebSocket(update, disconnect);
  const socket = FakeSocket.instances[0];
  const queuedMessage = socket.onmessage!;
  stop();
  queuedMessage({ data: JSON.stringify({ type: 'projects', data: [] }) });
  expect(update).not.toHaveBeenCalled();
  expect(disconnect).not.toHaveBeenCalled();
  expect(socket.onmessage).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
});

it('cancels a pending reconnect on cleanup', () => {
  const stop = subscribeProjectsWebSocket(vi.fn());
  FakeSocket.instances[0].close();
  expect(vi.getTimerCount()).toBe(1);
  stop();
  expect(vi.getTimerCount()).toBe(0);
  vi.advanceTimersByTime(15000);
  expect(FakeSocket.instances).toHaveLength(1);
});

it('backs off failed connections and resets after a successful connection', () => {
  const stop = subscribeProjectsWebSocket(vi.fn());
  FakeSocket.instances[0].close();
  vi.advanceTimersByTime(1000);
  expect(FakeSocket.instances).toHaveLength(2);
  FakeSocket.instances[1].close();
  vi.advanceTimersByTime(1999);
  expect(FakeSocket.instances).toHaveLength(2);
  vi.advanceTimersByTime(1);
  expect(FakeSocket.instances).toHaveLength(3);
  FakeSocket.instances[2].onopen?.();
  FakeSocket.instances[2].close();
  vi.advanceTimersByTime(1000);
  expect(FakeSocket.instances).toHaveLength(4);
  stop();
});

it('ignores invalid JSON and unrelated message types', () => {
  const update = vi.fn();
  const stop = subscribeProjectsWebSocket(update);
  for (const data of ['{', 'null', '{"type":"unknown"}', '{"type":"project","data":{}}']) {
    FakeSocket.instances[0].onmessage?.({ data });
  }
  expect(update).not.toHaveBeenCalled();
  FakeSocket.instances[0].onmessage?.({ data: '{"type":"projects","data":[]}' });
  expect(update).toHaveBeenCalledWith([]);
  stop();
});
