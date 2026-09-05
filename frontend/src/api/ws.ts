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

  const connect = () => {
    if (closed) return;
    ws = new WebSocket(buildWebSocketUrl(path));

    ws.onopen = () => {
      retryMs = 1000;
    };

    ws.onmessage = (event) => {
      const msg = parseMessage(String(event.data));
      if (msg) onMessage(msg);
    };

    ws.onerror = () => {
      ws?.close();
    };

    ws.onclose = () => {
      onDisconnect?.();
      if (closed) return;
      window.setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 15000);
    };
  };

  connect();

  return () => {
    closed = true;
    ws?.close();
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
