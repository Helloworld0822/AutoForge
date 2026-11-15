import { listProjects } from '../api/client';
import { subscribeProjectsWebSocket } from '../api/ws';
import type { Project } from '../types';
import { useLiveData } from './useLiveData';

const EMPTY_PROJECTS: Project[] = [];

export function useProjects() {
  const { data: projects, ...state } = useLiveData(
    listProjects,
    subscribeProjectsWebSocket,
    EMPTY_PROJECTS,
    '목록 로드 실패',
  );
  return { projects, ...state };
}
