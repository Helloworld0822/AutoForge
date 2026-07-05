import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { listProjects } from '../api/client';
import { ProjectCard } from '../components/ProjectCard';
import type { Project } from '../types';

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
    <div className="flex flex-col">
      <header className="sticky top-0 z-40 flex flex-wrap items-start justify-between gap-4 bg-bg px-8 py-8">
        <div className="flex flex-col">
          <h2 className="font-headline text-[2rem] font-bold leading-tight tracking-tight">
            대시보드
          </h2>
          <p className="mt-1 max-w-2xl text-sm text-muted">
            PDF 계획서 업로드부터 GitHub PR 머지까지 자동화된 파이프라인
          </p>
        </div>
        <div className="flex items-center gap-4">
          <button
            type="button"
            className="group flex items-center gap-2 rounded-lg border border-border px-4 py-2 text-foreground transition-colors hover:bg-surface-variant active:scale-95 disabled:cursor-not-allowed disabled:opacity-50"
            onClick={load}
            disabled={loading}
          >
            <span
              className={`material-symbols-outlined text-xl transition-transform duration-500 group-hover:rotate-180 ${loading ? 'animate-spin' : ''}`}
            >
              refresh
            </span>
            <span className="font-label text-xs tracking-wide">새로고침</span>
          </button>
          <Link
            to="/new"
            className="flex items-center gap-2 rounded-lg bg-accent px-6 py-2 font-bold text-[#002a65] shadow-lg shadow-accent/20 transition-all hover:opacity-90 active:scale-95"
          >
            <span className="material-symbols-outlined text-xl">add</span>
            <span className="font-label text-xs tracking-wide">새 프로젝트</span>
          </Link>
        </div>
      </header>

      <div className="mx-auto flex w-full max-w-7xl flex-col gap-12 px-8 pb-12">
        <section className="grid grid-cols-1 gap-5 md:grid-cols-3">
          <StatCard
            label="전체 프로젝트"
            value={projects.length}
            icon="folder"
            iconHover="group-hover:bg-accent group-hover:text-[#002d6c]"
            subtext="+0% from last month"
            subtextClass="text-secondary"
          />
          <StatCard
            label="실행 중"
            value={running}
            icon="sync"
            iconClass="text-secondary group-hover:bg-secondary group-hover:text-[#003824]"
            valueClass="text-primary"
            subtext="Active pipelines"
          />
          <StatCard
            label="완료"
            value={completed}
            icon="check_circle"
            iconClass="text-tertiary group-hover:bg-tertiary group-hover:text-[#472a00]"
            valueClass="text-secondary"
            subtext="Successfully merged"
          />
        </section>

        {error && (
          <div className="rounded-xl border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
            {error}
          </div>
        )}

        <section className="flex flex-col gap-6">
          <div className="flex items-center justify-between">
            <h3 className="font-headline text-xl font-semibold">프로젝트 목록</h3>
            <div className="flex gap-2">
              <button
                type="button"
                className="flex size-9 items-center justify-center rounded-lg border border-border bg-surface-container-high text-muted hover:text-foreground"
              >
                <span className="material-symbols-outlined text-xl">view_list</span>
              </button>
              <button
                type="button"
                className="flex size-9 items-center justify-center rounded-lg border border-border bg-surface-container-high text-muted hover:text-foreground"
              >
                <span className="material-symbols-outlined text-xl">grid_view</span>
              </button>
            </div>
          </div>

          <div className="relative min-h-[400px] overflow-hidden rounded-xl border border-border bg-surface-container-low">
            {loading && projects.length === 0 ? (
              <div className="flex h-[400px] items-center justify-center text-muted">로딩 중...</div>
            ) : projects.length === 0 ? (
              <div className="relative z-10 flex h-[400px] flex-col items-center justify-center px-6 text-center">
                <div className="mb-8 flex size-24 items-center justify-center rounded-full border-4 border-bg-elevated bg-surface-container-highest shadow-2xl">
                  <span className="material-symbols-outlined text-5xl text-outline">inbox</span>
                </div>
                <h4 className="font-headline mb-2 text-xl font-semibold">아직 프로젝트가 없습니다.</h4>
                <p className="mb-8 max-w-sm text-sm leading-relaxed text-muted">
                  자동화 프로세스를 시작하려면 새로운 프로젝트를 생성하고 워크플로우를 정의하세요.
                  PDF 명세서를 코드로 변환할 수 있습니다.
                </p>
                <Link
                  to="/new"
                  className="group inline-flex items-center gap-4 rounded-xl bg-accent px-8 py-4 font-bold text-[#002a65] shadow-xl shadow-accent/30 transition-all hover:-translate-y-0.5 hover:shadow-accent/40"
                >
                  <span className="font-label text-xs tracking-wide">첫 프로젝트 시작하기</span>
                  <span className="material-symbols-outlined transition-transform group-hover:translate-x-1">
                    arrow_forward
                  </span>
                </Link>
              </div>
            ) : (
              <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4 p-6">
                {projects.map((p) => (
                  <ProjectCard key={p.id} project={p} />
                ))}
              </div>
            )}
          </div>
        </section>

        <section className="grid grid-cols-1 gap-5 md:grid-cols-2">
          <InfoPanel
            icon="help"
            iconClass="bg-secondary/10 text-secondary"
            title="도움말 및 가이드"
            description={
              <>
                AutoForge를 처음 사용하시나요?{' '}
                <a href="https://github.com" className="text-primary hover:underline">
                  사용자 가이드
                </a>
                를 확인하세요.
              </>
            }
          />
          <InfoPanel
            icon="bolt"
            iconClass="bg-tertiary/10 text-tertiary"
            title="시스템 상태"
            description="현재 모든 서비스가 정상적으로 작동 중입니다."
          />
        </section>
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  icon,
  iconClass = 'text-primary group-hover:bg-accent group-hover:text-[#002d6c]',
  iconHover,
  valueClass = 'text-foreground',
  subtext,
  subtextClass = 'text-muted opacity-60',
}: {
  label: string;
  value: number;
  icon: string;
  iconClass?: string;
  iconHover?: string;
  valueClass?: string;
  subtext: string;
  subtextClass?: string;
}) {
  return (
    <div className="group flex flex-col gap-2 rounded-xl border border-border bg-surface-container-low p-6 transition-colors hover:border-accent/50">
      <div className="flex items-start justify-between">
        <span className="font-label text-xs uppercase tracking-wider text-muted">{label}</span>
        <div
          className={`flex size-8 items-center justify-center rounded-lg bg-surface-variant transition-colors ${iconClass} ${iconHover ?? ''}`}
        >
          <span className="material-symbols-outlined text-lg">{icon}</span>
        </div>
      </div>
      <div className="flex items-baseline gap-2">
        <span className={`font-headline text-[2rem] font-bold ${valueClass}`}>{value}</span>
        <span className={`font-label text-xs ${subtextClass}`}>{subtext}</span>
      </div>
    </div>
  );
}

function InfoPanel({
  icon,
  iconClass,
  title,
  description,
}: {
  icon: string;
  iconClass: string;
  title: string;
  description: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-4 rounded-xl border border-border/50 bg-bg-elevated p-6">
      <div className={`rounded-lg p-1 ${iconClass}`}>
        <span className="material-symbols-outlined">{icon}</span>
      </div>
      <div className="flex flex-col">
        <span className="font-label text-xs font-semibold">{title}</span>
        <span className="text-sm text-muted">{description}</span>
      </div>
    </div>
  );
}
