import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { listProjects } from '../api/client';
import { ProjectCard } from '../components/ProjectCard';
import type { Project } from '../types';
import { Plus, RefreshCw } from 'lucide-react';

export function DashboardPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      const data = await listProjects();
      setProjects(data);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : '목록 로드 실패');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  }, []);

  const running = projects.filter((p) => p.state === 'running').length;
  const completed = projects.filter((p) => p.state === 'completed').length;

  return (
    <div className="mx-auto max-w-[1100px]">
      <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
        <div>
          <h2 className="mt-1 text-[1.75rem] font-semibold">대시보드</h2>
          <p className="mt-1.5 max-w-xl text-muted">
            PDF 계획서 업로드부터 GitHub PR 머지까지 자동화된 파이프라인
          </p>
        </div>
        <div className="flex gap-2">
          <button
            className="inline-flex items-center justify-center gap-2 rounded-lg border border-border bg-transparent px-4 py-2.5 font-medium text-foreground transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
            onClick={load}
            disabled={loading}
          >
            <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
            새로고침
          </button>
          <Link
            to="/new"
            className="inline-flex items-center justify-center gap-2 rounded-lg bg-accent px-4 py-2.5 font-medium text-white transition-opacity hover:opacity-90"
          >
            <Plus size={16} />
            새 프로젝트
          </Link>
        </div>
      </header>

      <section className="mb-5 grid grid-cols-1 gap-4 md:grid-cols-3">
        <div className="rounded-lg border border-border bg-card p-4 md:p-5">
          <span className="mb-1.5 block text-sm text-muted">전체 프로젝트</span>
          <strong className="text-[1.75rem] font-semibold">{projects.length}</strong>
        </div>
        <div className="rounded-lg border border-border bg-card p-4 md:p-5">
          <span className="mb-1.5 block text-sm text-muted">실행 중</span>
          <strong className="text-[1.75rem] font-semibold text-accent">{running}</strong>
        </div>
        <div className="rounded-lg border border-border bg-card p-4 md:p-5">
          <span className="mb-1.5 block text-sm text-muted">완료</span>
          <strong className="text-[1.75rem] font-semibold text-success">{completed}</strong>
        </div>
      </section>

      {error && (
        <div className="mb-4 rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
          {error}
        </div>
      )}

      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <h3 className="mb-4 text-base font-medium">프로젝트 목록</h3>
        {loading && projects.length === 0 ? (
          <p className="text-muted">로딩 중...</p>
        ) : projects.length === 0 ? (
          <div className="py-8 text-center text-muted">
            <p>아직 프로젝트가 없습니다.</p>
            <Link
              to="/new"
              className="mt-4 inline-flex items-center justify-center gap-2 rounded-lg bg-accent px-4 py-2.5 font-medium text-white transition-opacity hover:opacity-90"
            >
              첫 프로젝트 시작하기
            </Link>
          </div>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4">
            {projects.map((p) => (
              <ProjectCard key={p.id} project={p} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
