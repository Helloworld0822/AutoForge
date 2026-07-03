import type {
  CreateProjectResponse,
  DailyLog,
  DailyLogSummary,
  HealthResponse,
  HostedImage,
  Project,
  ProjectDetail,
  UploadImageResponse,
} from '../types';

const API_BASE = import.meta.env.VITE_API_URL ?? '';
const API_KEY = import.meta.env.VITE_API_KEY as string | undefined;

function authHeaders(existing?: HeadersInit): HeadersInit {
  if (!API_KEY) return existing ?? {};
  return { ...(existing ?? {}), Authorization: `Bearer ${API_KEY}` };
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: authHeaders(init?.headers),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `HTTP ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export interface CreateProjectOptions {
  name?: string;
  repoUrl?: string;
  devopsPlanText?: string;
  devopsPlanFile?: File | null;
}

export async function getHealth(): Promise<HealthResponse> {
  return request<HealthResponse>('/health');
}

export async function listProjects(): Promise<Project[]> {
  return request<Project[]>('/v1/projects');
}

export async function getProject(id: string): Promise<ProjectDetail> {
  return request<ProjectDetail>(`/v1/projects/${id}`);
}

export async function createProject(
  planFile: File,
  options: CreateProjectOptions = {},
): Promise<CreateProjectResponse> {
  const form = new FormData();
  form.append('plan', planFile);
  if (options.name) form.append('name', options.name);
  if (options.repoUrl) form.append('repo_url', options.repoUrl);
  if (options.devopsPlanText?.trim()) {
    form.append('devops_plan_text', options.devopsPlanText.trim());
  }
  if (options.devopsPlanFile) {
    form.append('devops_plan', options.devopsPlanFile);
  }

  const res = await fetch(`${API_BASE}/v1/projects`, {
    method: 'POST',
    headers: authHeaders(),
    body: form,
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `HTTP ${res.status}`);
  }

  return res.json() as Promise<CreateProjectResponse>;
}

export async function getDailyLog(
  projectId: string,
  date: string,
): Promise<DailyLog> {
  return request<DailyLog>(`/v1/projects/${projectId}/daily-logs/${date}`);
}

export async function listDailyLogs(
  projectId: string,
): Promise<DailyLogSummary[]> {
  return request<DailyLogSummary[]>(`/v1/projects/${projectId}/daily-logs`);
}

export async function cancelProject(id: string): Promise<void> {
  await request(`/v1/projects/${id}/cancel`, { method: 'POST' });
}

export async function uploadImage(file: File): Promise<UploadImageResponse> {
  const form = new FormData();
  form.append('image', file);

  const res = await fetch(`${API_BASE}/v1/images`, {
    method: 'POST',
    headers: authHeaders(),
    body: form,
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `HTTP ${res.status}`);
  }

  return res.json() as Promise<UploadImageResponse>;
}

export async function listImages(): Promise<HostedImage[]> {
  return request<HostedImage[]>('/v1/images');
}

export function subscribeProjectStream(
  id: string,
  onUpdate: (data: unknown) => void,
  onError?: (err: Event) => void,
): () => void {
  const source = new EventSource(`${API_BASE}/v1/projects/${id}/stream`);
  source.addEventListener('status', (e) => {
    try {
      onUpdate(JSON.parse(e.data));
    } catch {
      onUpdate(e.data);
    }
  });
  source.onerror = (e) => {
    onError?.(e);
    source.close();
  };
  return () => source.close();
}
