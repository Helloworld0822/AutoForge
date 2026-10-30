import { Link, Outlet, useLocation } from 'react-router-dom';
import { useEffect, useState } from 'react';
import { getHealth } from '../api/client';
import { useAuth } from '../contexts/AuthContext';
import type { HealthResponse } from '../types';

const nav = [
  { to: '/', label: '대시보드', icon: 'dashboard' },
  { to: '/new', label: '새 프로젝트', icon: 'add_box' },
  { to: '/images', label: '이미지 호스팅', icon: 'image' },
  { to: '/estimates', label: '견적 작성', icon: 'request_quote' },
] as const;

const statusItems = [
  { key: 'api', label: 'API', icon: 'api', ok: (h: HealthResponse | null) => h?.status === 'ok' },
  {
    key: 'github',
    label: 'GitHub',
    icon: 'terminal',
    ok: (h: HealthResponse | null) => h?.github ?? false,
  },
  {
    key: 'merge',
    label: 'Auto Merge',
    icon: 'call_merge',
    ok: (h: HealthResponse | null) => h?.github_auto_merge ?? false,
  },
  {
    key: 'mq',
    label: 'MQ',
    icon: 'layers',
    ok: (h: HealthResponse | null) => h?.message_queue ?? false,
  },
] as const;

export function Layout() {
  const location = useLocation();
  const { user, logout } = useAuth();
  const [health, setHealth] = useState<HealthResponse | null>(null);

  useEffect(() => {
    getHealth().then(setHealth).catch(() => setHealth(null));
  }, []);

  return (
    <div className="flex h-dvh flex-col overflow-hidden md:flex-row">
      <aside className="z-50 flex w-full shrink-0 flex-col justify-between border-b border-border bg-bg-elevated px-4 py-4 md:h-dvh md:w-[260px] md:border-r md:border-b-0 md:py-8">
        <div className="flex flex-col gap-4 md:gap-8">
          <div className="flex items-center gap-4 px-2">
            <div className="flex size-10 items-center justify-center rounded-xl bg-accent text-[#002a65] shadow-lg shadow-accent/20">
              <span className="material-symbols-outlined material-symbols-filled text-xl">
                auto_mode
              </span>
            </div>
            <div className="flex flex-col">
              <h1 className="font-headline text-xl font-bold leading-tight">AutoForge</h1>
              <span className="font-label text-xs uppercase tracking-widest text-muted opacity-60">
                AI Automation Platform
              </span>
            </div>
          </div>

          <nav className="flex flex-wrap gap-1 md:flex-col">
            {nav.map(({ to, label, icon }) => {
              const active = location.pathname === to;
              return (
                <Link
                  key={to}
                  to={to}
                  className={`flex min-h-11 items-center gap-2 rounded-lg px-3 py-2 transition-colors md:gap-4 md:px-4 ${
                    active
                      ? 'bg-surface-variant text-primary'
                      : 'text-muted hover:bg-surface-variant'
                  }`}
                >
                  <span
                    className={`material-symbols-outlined text-xl ${active ? 'material-symbols-filled' : ''}`}
                  >
                    {icon}
                  </span>
                  <span className="text-sm font-semibold">{label}</span>
                </Link>
              );
            })}
          </nav>
        </div>

        <div className="flex flex-col gap-1 border-t border-border/30 pt-3 md:pt-6">
          {user?.session_login_enabled && user.authenticated && (
            <div className="mb-2 flex items-center justify-between rounded-lg px-4 py-2">
              <div className="flex flex-col gap-0.5">
                <span className="font-label text-[10px] uppercase tracking-widest text-muted opacity-60">
                  로그인
                </span>
                <span className="text-sm font-semibold text-foreground">{user.username}</span>
              </div>
              <button
                type="button"
                onClick={() => logout().catch(() => undefined)}
                className="rounded-lg px-3 py-1.5 text-xs font-semibold text-muted transition hover:bg-surface-variant hover:text-foreground"
              >
                로그아웃
              </button>
            </div>
          )}
          {statusItems.map(({ key, label, icon, ok }) => {
            const healthy = ok(health);
            return (
              <div
                key={key}
                className="hidden items-center justify-between rounded-lg px-4 py-2 text-muted transition-colors hover:bg-surface-variant md:flex"
              >
                <div className="flex items-center gap-4">
                  <span className="material-symbols-outlined text-xl">{icon}</span>
                  <span className="font-label text-xs tracking-wide">{label}</span>
                </div>
                <div
                  className={`size-2 rounded-full ${healthy ? 'bg-secondary' : 'bg-border'}`}
                />
              </div>
            );
          })}
        </div>
      </aside>

      <main className="custom-scrollbar min-h-0 min-w-0 flex-1 overflow-y-auto bg-bg">
        <Outlet />
      </main>
    </div>
  );
}
