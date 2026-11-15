import { useCallback } from 'react';
import { getProject } from '../api/client';
import { subscribeProjectWebSocket } from '../api/ws';
import type { ProjectDetail } from '../types';
import { useLiveData } from './useLiveData';

export function useProject(id: string | undefined) {
  const load = useCallback(() => getProject(id!), [id]);
  const subscribe = useCallback(
    (onUpdate: (data: ProjectDetail | null) => void) =>
      subscribeProjectWebSocket(id!, onUpdate),
    [id],
  );
  const { data: project, ...state } = useLiveData<ProjectDetail | null>(
    id ? load : null,
    subscribe,
    null,
    'Failed to load project',
  );
  return { project, ...state };
}
