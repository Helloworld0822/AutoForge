import {
  Activity,
  Box,
  FolderGit2,
  GitPullRequest,
  Image as ImageIcon,
  Layers,
  Sparkles,
  Upload,
  Zap,
} from 'lucide-react';
import { Link, Outlet, useLocation } from 'react-router-dom';
import { useEffect, useState } from 'react';
import { getHealth } from '../api/client';
import type { HealthResponse } from '../types';

export function Layout() {
  const location = useLocation();
  const [health, setHealth] = useState<HealthResponse | null>(null);

  useEffect(() => {
    getHealth().then(setHealth).catch(() => setHealth(null));
  }, []);

  const nav = [
    { to: '/', label: '대시보드', icon: Layers },
    { to: '/new', label: '새 프로젝트', icon: Upload },
    { to: '/images', label: '이미지 호스팅', icon: ImageIcon },
  ];

  return (
    <div className="grid min-h-screen grid-cols-1 md:grid-cols-[260px_1fr]">
      <aside className="flex flex-col gap-8 border-b border-border bg-bg-elevated p-6 md:border-b-0 md:border-r">
        <div className="flex items-center gap-3">
          <div className="grid size-10 place-items-center rounded-[10px] bg-accent-dim text-accent">
            <Sparkles size={20} />
          </div>
          <div>
            <h1 className="text-[1.1rem] font-bold">AutoForge</h1>
            <p className="text-xs text-muted">AI 외주 자동화</p>
          </div>
        </div>

        <nav className="flex flex-col gap-1.5">
          {nav.map(({ to, label, icon: Icon }) => {
            const active = location.pathname === to;
            return (
              <Link
                key={to}
                to={to}
                className={`flex items-center gap-2.5 rounded-lg px-3.5 py-2.5 text-muted transition-colors ${
                  active
                    ? 'bg-accent-dim text-foreground'
                    : 'hover:bg-accent-dim hover:text-foreground'
                }`}
              >
                <Icon size={18} />
                {label}
              </Link>
            );
          })}
        </nav>

        <div className="mt-auto">
          <div className="grid gap-2">
            <StatusPill
              icon={<Activity size={14} />}
              label="API"
              ok={health?.status === 'ok'}
            />
            <StatusPill
              icon={<FolderGit2 size={14} />}
              label="GitHub"
              ok={health?.github ?? false}
            />
            <StatusPill
              icon={<GitPullRequest size={14} />}
              label="Auto Merge"
              ok={health?.github_auto_merge ?? false}
            />
            <StatusPill
              icon={<Zap size={14} />}
              label="MQ"
              ok={health?.message_queue ?? false}
            />
          </div>
        </div>
      </aside>

      <main className="overflow-y-auto p-8">
        <Outlet />
      </main>
    </div>
  );
}

function StatusPill({
  icon,
  label,
  ok,
}: {
  icon: React.ReactNode;
  label: string;
  ok: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-1.5 rounded-md bg-card px-2 py-1.5 text-xs ${
        ok ? 'text-success' : 'text-muted'
      }`}
    >
      {icon}
      <span>{label}</span>
      <Box size={8} className="ml-auto fill-current" />
    </div>
  );
}
