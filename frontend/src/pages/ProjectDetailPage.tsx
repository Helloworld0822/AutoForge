import { Link, useParams } from 'react-router-dom';
import { useProject } from '../hooks/useProject';
import { ArchitectureQnAPanel } from '../components/ArchitectureQnAPanel';
import { PipelineStages } from '../components/PipelineStages';
import { DailyLogPanel } from '../components/DailyLogPanel';
import { cancelProject } from '../api/client';
import {
  ArrowLeft,
  ExternalLink,
  GitMerge,
  GitPullRequest,
  FolderGit2,
  Loader2,
  Server,
  StopCircle,
} from 'lucide-react';
import { useState } from 'react';

const stateLabel: Record<string, string> = {
  pending: '대기',
  running: '실행 중',
  awaiting_input: '입력 대기',
  completed: '완료',
  failed: '실패',
  cancelled: '취소됨',
};

const stateBadgeClass: Record<string, string> = {
  running: 'text-accent border-accent/40',
  awaiting_input: 'text-warn border-warn/40',
  completed: 'text-success border-success/40',
  failed: 'text-error border-error/40',
};

export function ProjectDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { project, loading, error, refresh } = useProject(id);
  const [cancelling, setCancelling] = useState(false);

  const handleCancel = async () => {
    if (!id || !confirm('이 프로젝트를 취소하시겠습니까?')) return;
    setCancelling(true);
    try {
      await cancelProject(id);
      await refresh();
    } finally {
      setCancelling(false);
    }
  };

  if (loading && !project) {
    return (
      <div className="mx-auto grid min-h-[60vh] max-w-[1100px] place-items-center">
        <Loader2 size={32} className="animate-spin text-accent" />
      </div>
    );
  }

  if (error || !project) {
    return (
      <div className="mx-auto max-w-[1100px]">
        <div className="mb-4 rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
          {error ?? '프로젝트를 찾을 수 없습니다.'}
        </div>
        <Link
          to="/"
          className="inline-flex items-center justify-center gap-2 rounded-lg border border-border bg-transparent px-4 py-2.5 font-medium text-foreground transition-opacity hover:opacity-90"
        >
          <ArrowLeft size={16} /> 대시보드로
        </Link>
      </div>
    );
  }

  const name = project.name || `Project ${project.id.slice(0, 8)}`;
  const outputs = project.stage_outputs ?? {};

  return (
    <div className="mx-auto max-w-[1100px]">
      <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
        <div>
          <Link
            to="/"
            className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted"
          >
            <ArrowLeft size={16} /> 대시보드
          </Link>
          <h2 className="text-[1.75rem] font-semibold">{name}</h2>
          <div className="mt-2 flex flex-wrap gap-2">
            <span
              className={`inline-block rounded-full border bg-bg px-2.5 py-0.5 text-xs font-medium ${
                stateBadgeClass[project.state] ?? 'text-muted border-border'
              }`}
            >
              {stateLabel[project.state] ?? project.state}
            </span>
            {project.has_devops_plan && (
              <span className="inline-flex items-center gap-1 rounded-full border border-violet-400/40 bg-bg px-2.5 py-0.5 text-xs font-medium text-violet-400">
                <Server size={12} /> DevOps
              </span>
            )}
            {(project.resolved_language || project.programming_language) && (
              <span className="inline-flex items-center gap-1 rounded-full border border-border bg-bg px-2.5 py-0.5 text-xs font-medium text-muted">
                {(project.resolved_language ?? project.programming_language)?.toUpperCase()}
              </span>
            )}
            <span className="inline-block rounded-full border border-border bg-bg px-2.5 py-0.5 text-xs font-medium text-muted">
              {project.id.slice(0, 8)}
            </span>
          </div>
        </div>
        {project.state === 'running' && (
          <button
            className="inline-flex items-center justify-center gap-2 rounded-lg border border-error/30 bg-transparent px-4 py-2.5 font-medium text-error transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
            onClick={handleCancel}
            disabled={cancelling}
          >
            <StopCircle size={16} />
            취소
          </button>
        )}
      </header>

      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <div>
          <div className="mb-2 flex justify-between">
            <span>전체 진행률</span>
            <strong>{project.progress_percent}%</strong>
          </div>
          <div className="h-2.5 overflow-hidden rounded-full bg-bg">
            <div
              className="h-full rounded-full bg-gradient-to-r from-accent to-violet-500 transition-[width] duration-400"
              style={{ width: `${project.progress_percent}%` }}
            />
          </div>
        </div>
      </section>

      {project.awaiting_architecture_input &&
        project.architecture_clarifications.length > 0 && (
          <ArchitectureQnAPanel
            projectId={project.id}
            clarifications={project.architecture_clarifications}
            onSubmitted={refresh}
          />
        )}

      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <h3 className="mb-4 text-base font-medium">파이프라인 스테이지</h3>
        <PipelineStages stages={project.stages} />
      </section>

      <DailyLogPanel projectId={project.id} />

      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <h3 className="mb-4 text-base font-medium">GitHub & 산출물</h3>
        <div className="grid gap-3">
          {project.repo_url && (
            <a
              href={project.repo_url}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-3.5 rounded-lg border border-border bg-bg px-4 py-3.5 transition-colors hover:border-accent"
            >
              <FolderGit2 size={20} />
              <div className="flex-1">
                <strong className="block text-sm">Repository</strong>
                <span className="text-xs text-muted">
                  {project.github_repo ?? project.repo_url}
                </span>
              </div>
              <ExternalLink size={16} className="text-muted" />
            </a>
          )}
          {project.pr_url && (
            <a
              href={project.pr_url}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-3.5 rounded-lg border border-border bg-bg px-4 py-3.5 transition-colors hover:border-accent"
            >
              <GitPullRequest size={20} />
              <div className="flex-1">
                <strong className="block text-sm">Pull Request</strong>
                <span className="text-xs text-muted">구현 PR</span>
              </div>
              <ExternalLink size={16} className="text-muted" />
            </a>
          )}
          {project.merge_status && (
            <div className="flex cursor-default items-center gap-3.5 rounded-lg border border-border bg-bg px-4 py-3.5">
              <GitMerge size={20} />
              <div className="flex-1">
                <strong className="block text-sm">Merge Status</strong>
                <span
                  className={`text-xs ${
                    project.merge_status === 'merged' ? 'text-success' : 'text-muted'
                  }`}
                >
                  {project.merge_status}
                </span>
              </div>
            </div>
          )}
        </div>
      </section>

      {Object.keys(outputs).length > 0 && (
        <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
          <h3 className="mb-4 text-base font-medium">스테이지 출력</h3>
          <div className="grid gap-2">
            {Object.entries(outputs).map(([stage, data]) => (
              <details
                key={stage}
                className="rounded-lg border border-border bg-bg px-3.5 py-2"
              >
                <summary className="cursor-pointer text-sm font-medium">{stage}</summary>
                <pre className="mt-3 overflow-x-auto text-[0.7rem] text-muted">
                  {JSON.stringify(data, null, 2)}
                </pre>
              </details>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
