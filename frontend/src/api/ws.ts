import type { Project, ProjectDetail } from '../types';

const API_BASE = import.meta.env.VITE_API_URL ?? '';
const API_KEY = import.meta.env.VITE_API_KEY as string | undefined;

export function buildWebSocketUrl(path: string): string {
  const origin =
    API_BASE ||
    `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}`;

  const base = origin.replace(/^http/i, 'ws');
  const url = new URL(path, base.endsWith('/') ? base : `${base}/`);

  if (API_KEY) {
    url.searchParams.set('access_token', API_KEY);
  }

  return url.toString();
}

type WsMessage =
  | { type: 'project'; data: ProjectDetail }
  | { type: 'projects'; data: Project[] };

function parseMessage(raw: string): WsMessage | null {
  try {
    const msg = JSON.parse(raw) as WsMessage;
    if (msg?.type === 'project' || msg?.type === 'projects') {
      return msg;
    }
  } catch {
    return null;
  }
  return null;
}

function connectWebSocket(
  path: string,
  onMessage: (msg: WsMessage) => void,
  onDisconnect?: () => void,
): () => void {
  let ws: WebSocket | null = null;
  let closed = false;
  let retryMs = 1000;
  let retryTimer: number | undefined;

  const detach = (socket: WebSocket) => {
    socket.onopen = null;
    socket.onmessage = null;
    socket.onerror = null;
    socket.onclose = null;
  };

  const connect = () => {
    retryTimer = undefined;
    if (closed) return;
    const socket = new WebSocket(buildWebSocketUrl(path));
    ws = socket;
    const isActive = () => !closed && ws === socket;

    socket.onopen = () => {
      if (!isActive()) return;
      retryMs = 1000;
    };

    socket.onmessage = (event) => {
      if (!isActive()) return;
      const msg = parseMessage(String(event.data));
      if (msg) onMessage(msg);
    };

    socket.onerror = () => {
      if (isActive()) socket.close();
    };

    socket.onclose = () => {
      if (!isActive()) return;
      detach(socket);
      ws = null;
      onDisconnect?.();
      if (closed) return;
      retryTimer = window.setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 15000);
    };
  };

  connect();

  return () => {
    if (closed) return;
    closed = true;
    if (retryTimer !== undefined) {
      window.clearTimeout(retryTimer);
      retryTimer = undefined;
    }
    if (ws) {
      detach(ws);
      ws.close();
      ws = null;
    }
  };
}

export function subscribeProjectWebSocket(
  id: string,
  onUpdate: (project: ProjectDetail) => void,
  onDisconnect?: () => void,
): () => void {
  return connectWebSocket(
    `/v1/projects/${id}/ws`,
    (msg) => {
      if (msg.type === 'project') onUpdate(msg.data);
    },
    onDisconnect,
  );
}

export function subscribeProjectsWebSocket(
  onUpdate: (projects: Project[]) => void,
  onDisconnect?: () => void,
): () => void {
  return connectWebSocket(
    '/v1/projects/ws',
    (msg) => {
      if (msg.type === 'projects') onUpdate(msg.data);
    },
    onDisconnect,
  );
}
