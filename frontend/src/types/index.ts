export type PipelineState = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';

export type StageId =
  | 'ingest'
  | 'summarize'
  | 'architect'
  | 'design'
  | 'implement'
  | 'verify'
  | 'debug'
  | 'security_patch'
  | 'deliver';

export type StageState = 'queued' | 'running' | 'completed' | 'failed' | 'skipped';

export interface StageStatus {
  stage: StageId;
  status: StageState;
}

export interface Project {
  id: string;
  name: string | null;
  repo_url: string | null;
  state: PipelineState;
  stages: StageStatus[];
  progress_percent: number;
  pr_url: string | null;
  merge_status: string | null;
  github_repo: string | null;
  has_devops_plan: boolean;
  created_at: string;
}

export interface ProjectDetail extends Project {
  stage_outputs: Record<string, unknown>;
}

export interface DailyLogSummary {
  date: string;
  day_number: number;
  entry_count: number;
  progress_percent: number;
  updated_at: string;
}

export interface DailyLog extends DailyLogSummary {
  entries: {
    at: string;
    event: string;
    stage?: string;
    message: string;
    progress_percent: number;
  }[];
  markdown: string;
}

export interface CreateProjectResponse {
  id: string;
  state: PipelineState;
  repo_url: string | null;
  message: string;
  mode: string;
  stream_url: string;
  progress_percent: number;
  github_auto_created: boolean;
  has_devops_plan: boolean;
}

export interface HealthResponse {
  status: string;
  service: string;
  message_queue: boolean;
  slack: boolean;
  github: boolean;
  github_auto_merge: boolean;
}

export const STAGE_META: Record<
  StageId,
  { label: string; description: string; model?: string }
> = {
  ingest: { label: 'Ingest', description: 'PDF 계획서 파싱 및 저장' },
  summarize: {
    label: 'Summarize',
    description: '계획서 구조화 요약',
    model: 'Sonnet',
  },
  architect: {
    label: 'Architect',
    description: '시스템 아키텍처 & 상세 기획',
    model: 'Fable',
  },
  design: { label: 'Design', description: 'UI 디자인 생성', model: 'Stitch' },
  implement: {
    label: 'Implement',
    description: '코드 구현 & PR 생성',
    model: 'Codex 5.3',
  },
  verify: {
    label: 'Verify',
    description: '테스트·린트·빌드 검증',
    model: 'Codex 5.3',
  },
  debug: {
    label: 'Debug',
    description: '검증 실패 자동 수정',
    model: 'Codex 5.3',
  },
  security_patch: {
    label: 'Security',
    description: '보안 감사 & 패치',
    model: 'Fable',
  },
  deliver: { label: 'Deliver', description: 'PR 머지 & 산출물 배포' },
};
