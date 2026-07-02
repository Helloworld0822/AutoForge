import { Link, useParams } from 'react-router-dom';
import { useProject } from '../hooks/useProject';
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
  completed: '완료',
  failed: '실패',
  cancelled: '취소됨',
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
      <div className="page center">
        <Loader2 size={32} className="spin accent" />
      </div>
    );
  }

  if (error || !project) {
    return (
      <div className="page">
        <div className="alert error">{error ?? '프로젝트를 찾을 수 없습니다.'}</div>
        <Link to="/" className="btn ghost">
          <ArrowLeft size={16} /> 대시보드로
        </Link>
      </div>
    );
  }

  const name = project.name || `Project ${project.id.slice(0, 8)}`;
  const outputs = project.stage_outputs ?? {};

  return (
    <div className="page">
      <header className="page-header">
        <div>
          <Link to="/" className="back-link">
            <ArrowLeft size={16} /> 대시보드
          </Link>
          <h2>{name}</h2>
          <div className="header-badges">
            <span className={`badge state-${project.state}`}>
              {stateLabel[project.state] ?? project.state}
            </span>
            {project.has_devops_plan && (
              <span className="badge devops">
                <Server size={12} /> DevOps
              </span>
            )}
            <span className="badge muted">{project.id.slice(0, 8)}</span>
          </div>
        </div>
        {project.state === 'running' && (
          <button
            className="btn danger ghost"
            onClick={handleCancel}
            disabled={cancelling}
          >
            <StopCircle size={16} />
            취소
          </button>
        )}
      </header>

      <section className="card">
        <div className="detail-progress">
          <div className="progress-header">
            <span>전체 진행률</span>
            <strong>{project.progress_percent}%</strong>
          </div>
          <div className="progress-bar large">
            <div
              className="progress-fill"
              style={{ width: `${project.progress_percent}%` }}
            />
          </div>
        </div>
      </section>

      <section className="card">
        <h3>파이프라인 스테이지</h3>
        <PipelineStages stages={project.stages} />
      </section>

      <DailyLogPanel projectId={project.id} />

      <section className="card links-card">
        <h3>GitHub & 산출물</h3>
        <div className="link-grid">
          {project.repo_url && (
            <a
              href={project.repo_url}
              target="_blank"
              rel="noreferrer"
              className="link-tile"
            >
              <FolderGit2 size={20} />
              <div>
                <strong>Repository</strong>
                <span>{project.github_repo ?? project.repo_url}</span>
              </div>
              <ExternalLink size={16} className="muted" />
            </a>
          )}
          {project.pr_url && (
            <a
              href={project.pr_url}
              target="_blank"
              rel="noreferrer"
              className="link-tile"
            >
              <GitPullRequest size={20} />
              <div>
                <strong>Pull Request</strong>
                <span>구현 PR</span>
              </div>
              <ExternalLink size={16} className="muted" />
            </a>
          )}
          {project.merge_status && (
            <div className="link-tile static">
              <GitMerge size={20} />
              <div>
                <strong>Merge Status</strong>
                <span
                  className={
                    project.merge_status === 'merged' ? 'success' : 'muted'
                  }
                >
                  {project.merge_status}
                </span>
              </div>
            </div>
          )}
        </div>
      </section>

      {Object.keys(outputs).length > 0 && (
        <section className="card">
          <h3>스테이지 출력</h3>
          <div className="outputs-grid">
            {Object.entries(outputs).map(([stage, data]) => (
              <details key={stage} className="output-block">
                <summary>{stage}</summary>
                <pre>{JSON.stringify(data, null, 2)}</pre>
              </details>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
