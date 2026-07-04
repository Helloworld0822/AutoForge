import { useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { createProject } from '../api/client';
import { ModelConfigPanel } from '../components/ModelConfigPanel';
import type { PipelineModelConfig } from '../types';

const DEVOPS_ACCEPT =
  '.md,.markdown,.yaml,.yml,.txt,.pdf,application/pdf,text/markdown,text/plain';

const DEVOPS_PLACEHOLDER = `# DevOps 계획서 예시

## 인프라
- Docker Compose (nginx + api + redis)
- Podman 호환

## CI/CD
- GitHub Actions: lint → test → build → deploy
- PR 머지 시 자동 배포

## 모니터링
- 헬스체크 /health
- Slack 알림

## 보안
- 시크릿은 GitHub Secrets / 환경변수로 관리
`;

export function NewProjectPage() {
  const navigate = useNavigate();
  const fileRef = useRef<HTMLInputElement>(null);
  const devopsFileRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [file, setFile] = useState<File | null>(null);
  const [devopsFile, setDevopsFile] = useState<File | null>(null);
  const [devopsText, setDevopsText] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [modelConfig, setModelConfig] = useState<PipelineModelConfig>({});

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const dropped = e.dataTransfer.files[0];
    if (dropped?.type === 'application/pdf') setFile(dropped);
    else setError('PDF 파일만 업로드할 수 있습니다.');
  };

  const hasDevops = devopsText.trim().length > 0 || devopsFile !== null;

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) {
      setError('PDF 계획서를 선택해주세요.');
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await createProject(file, {
        name: name || undefined,
        repoUrl: repoUrl || undefined,
        devopsPlanText: devopsText || undefined,
        devopsPlanFile: devopsFile,
        modelConfig,
      });
      navigate(`/projects/${res.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : '프로젝트 생성 실패');
    } finally {
      setLoading(false);
    }
  };

  const dropzoneClass = (active: boolean, hasFile: boolean) =>
    `mb-5 cursor-pointer rounded-xl border-2 border-dashed p-10 text-center transition-colors ${
      active || hasFile
        ? 'border-accent bg-accent-dim'
        : 'border-border hover:border-accent/50 hover:bg-accent-dim'
    } ${hasFile ? 'border-solid' : ''}`;

  return (
    <div className="flex flex-col">
      <header className="sticky top-0 z-40 border-b border-border bg-bg px-8 py-6">
        <h2 className="font-headline text-[2rem] font-bold leading-tight tracking-tight">
          새 프로젝트
        </h2>
        <p className="mt-1 max-w-xl text-sm text-muted">
          PDF 외주 계획서와 함께 DevOps 계획서(CI/CD, 인프라, 배포)를 작성하거나
          업로드할 수 있습니다.
        </p>
      </header>

      <div className="mx-auto w-full max-w-4xl px-8 py-8">
        <ModelConfigPanel value={modelConfig} onChange={setModelConfig} />

        <form
          className="mb-8 rounded-xl border border-border bg-surface-container-low p-6 md:p-8"
          onSubmit={onSubmit}
        >
          <SectionTitle icon="description" title="외주 계획서 (PDF) — 필수" />
          <div
            className={dropzoneClass(dragOver, !!file)}
            onDragOver={(e) => {
              e.preventDefault();
              setDragOver(true);
            }}
            onDragLeave={() => setDragOver(false)}
            onDrop={onDrop}
            onClick={() => fileRef.current?.click()}
          >
            <input
              ref={fileRef}
              type="file"
              accept="application/pdf"
              hidden
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            />
            {file ? (
              <>
                <span className="material-symbols-outlined text-5xl text-accent">description</span>
                <strong className="mt-3 block">{file.name}</strong>
                <span className="text-muted">{(file.size / 1024).toFixed(1)} KB</span>
              </>
            ) : (
              <>
                <span className="material-symbols-outlined text-5xl text-muted">upload_file</span>
                <strong className="mt-3 block">PDF 계획서를 드래그하거나 클릭하여 선택</strong>
                <span className="text-muted">필수 · 최대 50MB</span>
              </>
            )}
          </div>

          <SectionTitle icon="dns" title="DevOps 계획서 — 선택" />
          <p className="mb-4 text-sm text-muted">
            CI/CD, Docker/K8s, 인프라, 모니터링, 배포 전략을 직접 작성하거나
            파일(.md, .yaml, .yml, .txt, .pdf)로 업로드하세요.
          </p>

          <label className="mb-4 block text-sm text-muted">
            직접 작성
            <textarea
              rows={10}
              placeholder={DEVOPS_PLACEHOLDER}
              value={devopsText}
              onChange={(e) => setDevopsText(e.target.value)}
              className="code-editor mt-2 block min-h-40 w-full resize-y rounded-lg border border-border bg-surface-container-lowest px-4 py-3 font-label text-sm leading-relaxed text-foreground outline-none focus:border-accent"
            />
          </label>

          <div
            className={`${dropzoneClass(false, !!devopsFile)} mb-4 p-5`}
            onClick={() => devopsFileRef.current?.click()}
          >
            <input
              ref={devopsFileRef}
              type="file"
              accept={DEVOPS_ACCEPT}
              hidden
              onChange={(e) => setDevopsFile(e.target.files?.[0] ?? null)}
            />
            {devopsFile ? (
              <>
                <span className="material-symbols-outlined text-3xl text-accent">cloud_upload</span>
                <strong className="mt-2 block">{devopsFile.name}</strong>
                <button
                  type="button"
                  className="mt-2 inline-flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs transition-opacity hover:opacity-90"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDevopsFile(null);
                  }}
                >
                  제거
                </button>
              </>
            ) : (
              <>
                <span className="material-symbols-outlined text-3xl text-muted">cloud_upload</span>
                <strong className="mt-2 block">DevOps 계획서 파일 업로드 (선택)</strong>
                <span className="text-muted">.md · .yaml · .yml · .txt · .pdf</span>
              </>
            )}
          </div>

          {hasDevops && (
            <div className="mb-4 rounded-lg border border-accent/30 bg-accent/10 px-4 py-3 text-sm text-primary">
              DevOps 계획서가 파이프라인에 반영됩니다 — CI/CD, Dockerfile, compose,
              인프라 설정이 자동 생성됩니다.
            </div>
          )}

          <label className="mb-4 block text-sm text-muted">
            프로젝트 이름
            <input
              type="text"
              placeholder="예: 쇼핑몰 MVP"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="mt-2 block w-full rounded-lg border border-border bg-surface-container-lowest px-4 py-2.5 text-foreground outline-none focus:border-accent"
            />
          </label>

          <label className="mb-4 block text-sm text-muted">
            <span className="inline-flex items-center gap-2">
              <span className="material-symbols-outlined text-base">terminal</span>
              GitHub Repo URL (선택)
            </span>
            <input
              type="url"
              placeholder="비워두면 프라이빗 레포 자동 생성"
              value={repoUrl}
              onChange={(e) => setRepoUrl(e.target.value)}
              className="mt-2 block w-full rounded-lg border border-border bg-surface-container-lowest px-4 py-2.5 text-foreground outline-none focus:border-accent"
            />
            <small className="mt-2 block text-xs text-muted">
              GITHUB_TOKEN 환경 변수가 설정되어 있으면 자동으로 프라이빗 레포를 만들고,
              검증 통과 후 PR을 머지합니다.
            </small>
          </label>

          {error && (
            <div className="mb-4 rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
              {error}
            </div>
          )}

          <button
            type="submit"
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3.5 font-bold text-[#002a65] shadow-lg shadow-accent/20 transition-all hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading}
          >
            {loading ? (
              <>
                <span className="material-symbols-outlined animate-spin text-xl">progress_activity</span>
                파이프라인 시작 중...
              </>
            ) : (
              <>
                <span className="material-symbols-outlined text-xl">play_arrow</span>
                파이프라인 시작
              </>
            )}
          </button>
        </form>

        <section className="rounded-xl border border-border bg-bg-elevated p-6 md:p-8">
          <h3 className="mb-4 font-headline text-base font-semibold">자동화 파이프라인</h3>
          <ol className="list-decimal space-y-2 pl-5 text-sm text-muted">
            <li>
              <strong className="text-foreground">Summarize</strong> — Haiku가 PDF + DevOps
              계획서 통합 요약
            </li>
            <li>
              <strong className="text-foreground">Architect</strong> — Sonnet이 아키텍처 &
              인프라/CI/CD 설계
            </li>
            <li>
              <strong className="text-foreground">Design</strong> — Stitch로 UI 디자인
            </li>
            <li>
              <strong className="text-foreground">Implement</strong> — Codex 5.3이 코드 + DevOps
              산출물 구현
            </li>
            <li>
              <strong className="text-foreground">Verify → Debug</strong> — 품질 게이트 (최대 3회)
            </li>
            <li>
              <strong className="text-foreground">Security → Deliver</strong> — 보안 패치 후 PR
              자동 머지
            </li>
          </ol>
        </section>
      </div>
    </div>
  );
}

function SectionTitle({ icon, title }: { icon: string; title: string }) {
  return (
    <h3 className="mb-3 mt-1 flex items-center gap-2 text-sm font-medium">
      <span className="material-symbols-outlined text-lg text-accent">{icon}</span>
      {title}
    </h3>
  );
}
