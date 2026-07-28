import { useCallback, useEffect, useRef, useState } from 'react';
import { getProject } from '../api/client';
import { subscribeProjectWebSocket } from '../api/ws';
import type { ProjectDetail } from '../types';

export function useProject(id: string | undefined) {
  const [project, setProject] = useState<ProjectDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const wsActive = useRef(false);

  const refresh = useCallback(async () => {
    if (!id) return;
    try {
      const data = await getProject(id);
      setProject(data);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load project');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    if (!id) return;

    setLoading(true);
    refresh();

    if (typeof WebSocket === 'undefined') {
      return;
    }

    wsActive.current = true;
    const cleanup = subscribeProjectWebSocket(
      id,
      (data) => {
        setProject(data);
        setError(null);
        setLoading(false);
      },
      () => {
        wsActive.current = false;
      },
    );

    return () => {
      cleanup();
      wsActive.current = false;
    };
  }, [id, refresh]);

  return { project, loading, error, refresh };
}
