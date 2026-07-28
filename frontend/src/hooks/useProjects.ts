import { useCallback, useEffect, useRef, useState } from 'react';
import { listProjects } from '../api/client';
import { subscribeProjectsWebSocket } from '../api/ws';
import type { Project } from '../types';

export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const wsActive = useRef(false);

  const refresh = useCallback(async () => {
    try {
      const data = await listProjects();
      setProjects(data);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : '목록 로드 실패');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();

    if (typeof WebSocket !== 'undefined') {
      wsActive.current = true;
      const cleanup = subscribeProjectsWebSocket(
        (data) => {
          setProjects(data);
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
    }
  }, [refresh]);

  return { projects, loading, error, refresh };
}
