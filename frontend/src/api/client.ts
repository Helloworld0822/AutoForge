import type {
  Estimate,
  EstimateInput,
  EstimateGenerationResult,
  AuthMeResponse,
  CreateProjectResponse,
  DailyLog,
  DailyLogSummary,
  HealthResponse,
  HostedImage,
  LanguageMode,
  ModelsListResponse,
  PipelineModelConfig,
  ProgrammingLanguage,
  Project,
  ProjectDetail,
  UploadImageResponse,
} from '../types';

const API_BASE = import.meta.env.VITE_API_URL ?? '';
const API_KEY = import.meta.env.VITE_API_KEY as string | undefined;

export function listEstimates(): Promise<Estimate[]> {
  return request('/v1/estimates');
}

export function saveEstimate(draft: EstimateInput, id: string | null): Promise<Estimate> {
  return request(id ? `/v1/estimates/${id}` : '/v1/estimates', {
    method: id ? 'PUT' : 'POST',
    body: JSON.stringify(draft),
    headers: { 'Content-Type': 'application/json' },
  });
}

export function generateEstimate(input: { requirements: string; daily_rate: number; title: string }): Promise<{ id: string }> {
  return request('/v1/estimates/generate', {
    method: 'POST', body: JSON.stringify(input), headers: { 'Content-Type': 'application/json' },
  });
}

export function getEstimateGeneration(id: string): Promise<EstimateGenerationResult> {
  return request(`/v1/estimates/generations/${id}`);
}

function authHeaders(existing?: HeadersInit): HeadersInit {
  if (!API_KEY) return existing ?? {};
  return { ...(existing ?? {}), Authorization: `Bearer ${API_KEY}` };
}

function fetchOptions(init?: RequestInit): RequestInit {
  return {
    credentials: 'include',
    ...init,
    headers: authHeaders(init?.headers),
  };
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, fetchOptions(init));
  if (!res.ok) {
    const body = await res.text();
    let message = body;
    try {
      const parsed: unknown = JSON.parse(body);
      if (typeof parsed === 'object' && parsed !== null && 'error' in parsed && typeof parsed.error === 'string') message = parsed.error;
    } catch (error) {
      if (!(error instanceof SyntaxError)) throw error;
    }
    throw new Error(message || `HTTP ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export interface CreateProjectOptions {
  name?: string;
  repoUrl?: string;
  devopsPlanText?: string;
  devopsPlanFile?: File | null;
  programmingLanguage?: ProgrammingLanguage;
  languageMode?: LanguageMode;
  modelConfig?: PipelineModelConfig;
}

export async function getHealth(): Promise<HealthResponse> {
  return request<HealthResponse>('/health');
}

export async function getAuthMe(): Promise<AuthMeResponse> {
  return request<AuthMeResponse>('/v1/auth/me');
}

export async function login(username: string, password: string): Promise<void> {
  await request('/v1/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });
}

export async function logout(): Promise<void> {
  await request('/v1/auth/logout', { method: 'POST' });
}

export async function listProjects(): Promise<Project[]> {
  return request<Project[]>('/v1/projects');
}

export async function getProject(id: string): Promise<ProjectDetail> {
  return request<ProjectDetail>(`/v1/projects/${id}`);
}

export async function listModels(): Promise<ModelsListResponse> {
  return request<ModelsListResponse>('/v1/models');
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
  if (options.languageMode) {
    form.append('language_mode', options.languageMode);
  }
  if (options.programmingLanguage) {
    form.append('programming_language', options.programmingLanguage);
  }
  if (options.modelConfig && Object.keys(options.modelConfig).length > 0) {
    form.append('model_config', JSON.stringify(options.modelConfig));
  }

  const res = await fetch(`${API_BASE}/v1/projects`, fetchOptions({
    method: 'POST',
    body: form,
  }));

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

export interface RestartProjectOptions {
  modelConfig?: PipelineModelConfig;
  fromStage?: string;
}

export async function restartProject(
  id: string,
  options: RestartProjectOptions = {},
): Promise<void> {
  await request(`/v1/projects/${id}/restart`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      model_config: options.modelConfig,
      from_stage: options.fromStage,
    }),
  });
}

export async function submitArchitectureAnswers(
  projectId: string,
  answers: { id: string; answer: string }[],
): Promise<void> {
  await request(`/v1/projects/${projectId}/architecture-answers`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ answers }),
  });
}

export const PROGRAMMING_LANGUAGES: {
  value: ProgrammingLanguage;
  label: string;
}[] = [
  { value: 'rust', label: 'Rust' },
  { value: 'typescript', label: 'TypeScript' },
  { value: 'python', label: 'Python' },
  { value: 'go', label: 'Go' },
  { value: 'java', label: 'Java' },
  { value: 'kotlin', label: 'Kotlin' },
  { value: 'swift', label: 'Swift' },
  { value: 'csharp', label: 'C#' },
  { value: 'ruby', label: 'Ruby' },
  { value: 'php', label: 'PHP' },
];

export async function uploadImage(file: File): Promise<UploadImageResponse> {
  const form = new FormData();
  form.append('image', file);

  const res = await fetch(`${API_BASE}/v1/images`, fetchOptions({
    method: 'POST',
    body: form,
  }));

  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `HTTP ${res.status}`);
  }

  return res.json() as Promise<UploadImageResponse>;
}

export async function listImages(): Promise<HostedImage[]> {
  return request<HostedImage[]>('/v1/images');
}
