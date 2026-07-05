import { type FormEvent, useState } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '../contexts/AuthContext';

export function LoginPage() {
  const { login, loginRequired, loading } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? '/';

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  if (!loading && !loginRequired) {
    return <Navigate to={from} replace />;
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      await login(username.trim(), password);
      navigate(from, { replace: true });
    } catch (err) {
      const msg = err instanceof Error ? err.message : '로그인에 실패했습니다';
      try {
        const parsed = JSON.parse(msg) as { error?: string };
        setError(parsed.error ?? msg);
      } catch {
        setError(msg);
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-bg px-4">
      <div className="w-full max-w-md rounded-2xl border border-border bg-bg-elevated p-8 shadow-2xl">
        <div className="mb-8 flex flex-col items-center gap-3 text-center">
          <div className="flex size-14 items-center justify-center rounded-2xl bg-accent text-[#002a65] shadow-lg shadow-accent/20">
            <span className="material-symbols-outlined material-symbols-filled text-3xl">
              auto_mode
            </span>
          </div>
          <h1 className="font-headline text-2xl font-bold">AutoForge</h1>
          <p className="text-sm text-muted">아이디와 비밀번호로 로그인하세요</p>
        </div>

        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
          <label className="flex flex-col gap-2">
            <span className="font-label text-xs uppercase tracking-widest text-muted">아이디</span>
            <input
              type="text"
              autoComplete="username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="rounded-lg border border-border bg-surface-container-low px-4 py-3 text-foreground outline-none transition focus:border-accent"
              required
            />
          </label>

          <label className="flex flex-col gap-2">
            <span className="font-label text-xs uppercase tracking-widest text-muted">비밀번호</span>
            <input
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="rounded-lg border border-border bg-surface-container-low px-4 py-3 text-foreground outline-none transition focus:border-accent"
              required
            />
          </label>

          {error && (
            <p className="rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
              {error}
            </p>
          )}

          <button
            type="submit"
            disabled={submitting || loading}
            className="mt-2 rounded-lg bg-accent px-4 py-3 font-semibold text-[#002a65] transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {submitting ? '로그인 중…' : '로그인'}
          </button>
        </form>
      </div>
    </div>
  );
}
