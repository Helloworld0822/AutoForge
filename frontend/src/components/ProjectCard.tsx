import { Link } from 'react-router-dom';
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
  running: 'text-primary border-primary/40 bg-primary/10',
  awaiting_input: 'text-warn border-warn/40 bg-warn/10',
  completed: 'text-secondary border-secondary/40 bg-secondary/10',
  failed: 'text-error border-error/40 bg-error/10',
};

export function ProjectCard({ project }: { project: Project }) {
  const name = project.name || `Project ${project.id.slice(0, 8)}`;

  return (
    <Link
      to={`/projects/${project.id}`}
      className="group block rounded-xl border border-border bg-bg-elevated p-5 transition-all hover:border-accent/50 hover:shadow-lg hover:shadow-accent/5"
    >
      <div className="mb-3 flex items-start justify-between">
        <div>
          <h3 className="mb-2 font-headline text-base font-semibold">{name}</h3>
          <span
            className={`inline-block rounded-full border px-2.5 py-0.5 font-label text-xs ${
              stateBadgeClass[project.state] ?? 'border-border text-muted'
            }`}
          >
            {stateLabel[project.state] ?? project.state}
          </span>
        </div>
        <span className="material-symbols-outlined text-muted transition-transform group-hover:translate-x-0.5 group-hover:text-primary">
          arrow_forward
        </span>
      </div>

      <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-lowest">
        <div
          className="h-full rounded-full bg-gradient-to-r from-accent to-secondary transition-[width] duration-400"
          style={{ width: `${project.progress_percent}%` }}
        />
      </div>
      <p className="mt-2 font-label text-xs text-muted">{project.progress_percent}% 완료</p>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {project.github_repo && (
          <Tag icon="terminal" label={project.github_repo} />
        )}
        {project.pr_url && <Tag icon="call_merge" label="PR" />}
        {project.merge_status === 'merged' && (
          <span className="inline-flex items-center gap-1 rounded-md bg-secondary/10 px-2 py-0.5 font-label text-xs text-secondary">
            Merged
          </span>
        )}
        {project.has_devops_plan && <Tag icon="dns" label="DevOps" />}
      </div>
    </Link>
  );
}

function Tag({ icon, label }: { icon: string; label: string }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 truncate rounded-md bg-surface-container-high px-2 py-0.5 font-label text-xs text-muted">
      <span className="material-symbols-outlined text-sm">{icon}</span>
      <span className="truncate">{label}</span>
    </span>
  );
}
