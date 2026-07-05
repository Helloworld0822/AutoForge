import { Link } from 'react-router-dom';
import { ArrowRight, FolderGit2, GitPullRequest, Server } from 'lucide-react';
import type { Project } from '../types';

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

export function ProjectCard({ project }: { project: Project }) {
  const name = project.name || `Project ${project.id.slice(0, 8)}`;

  return (
    <Link
      to={`/projects/${project.id}`}
      className="block rounded-[10px] border border-border bg-bg p-4 transition-colors hover:border-accent"
    >
      <div className="mb-3 flex items-start justify-between">
        <div>
          <h3 className="mb-1.5 text-[0.95rem] font-medium">{name}</h3>
          <span
            className={`inline-block rounded-full border bg-bg px-2.5 py-0.5 text-xs font-medium ${
              stateBadgeClass[project.state] ?? 'text-muted border-border'
            }`}
          >
            {stateLabel[project.state] ?? project.state}
          </span>
        </div>
        <ArrowRight size={18} className="text-muted" />
      </div>

      <div className="h-1.5 overflow-hidden rounded-full bg-bg">
        <div
          className="h-full rounded-full bg-gradient-to-r from-accent to-violet-500 transition-[width] duration-400"
          style={{ width: `${project.progress_percent}%` }}
        />
      </div>
      <p className="mt-1.5 text-xs text-muted">{project.progress_percent}% 완료</p>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {project.github_repo && (
          <span className="inline-flex items-center gap-1 rounded-md bg-card px-1.5 py-0.5 text-[0.7rem] text-muted">
            <FolderGit2 size={14} />
            {project.github_repo}
          </span>
        )}
        {project.pr_url && (
          <span className="inline-flex items-center gap-1 rounded-md bg-card px-1.5 py-0.5 text-[0.7rem] text-muted">
            <GitPullRequest size={14} />
            PR
          </span>
        )}
        {project.merge_status === 'merged' && (
          <span className="inline-flex items-center gap-1 rounded-md bg-card px-1.5 py-0.5 text-[0.7rem] text-success">
            Merged
          </span>
        )}
        {project.has_devops_plan && (
          <span className="inline-flex items-center gap-1 rounded-md bg-card px-1.5 py-0.5 text-[0.7rem] text-muted">
            <Server size={14} />
            DevOps
          </span>
        )}
      </div>
    </Link>
  );
}
