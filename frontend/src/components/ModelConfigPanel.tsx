import { useEffect, useState } from 'react';
import { listModels } from '../api/client';
import type { CursorModel, PipelineModelConfig, StageId } from '../types';

const STAGE_FIELDS: {
  key: keyof PipelineModelConfig;
  stage: StageId;
  label: string;
  description: string;
}[] = [
  {
    key: 'summarize',
    stage: 'summarize',
    label: 'Summarize',
    description: 'PDF 계획서 요약',
  },
  {
    key: 'architect',
    stage: 'architect',
    label: 'Architect',
    description: '아키텍처 & 기획',
  },
  {
    key: 'implement',
    stage: 'implement',
    label: 'Implement',
    description: '코드 구현 & PR',
  },
  {
    key: 'verify',
    stage: 'verify',
    label: 'Verify',
    description: '테스트·린트·빌드',
  },
  {
    key: 'debug',
    stage: 'debug',
    label: 'Debug',
    description: '검증 실패 수정',
  },
  {
    key: 'security_patch',
    stage: 'security_patch',
    label: 'Security',
    description: '보안 감사 & 패치',
  },
];

interface ModelConfigPanelProps {
  value: PipelineModelConfig;
  onChange: (value: PipelineModelConfig) => void;
}

export function ModelConfigPanel({ value, onChange }: ModelConfigPanelProps) {
  const [expanded, setExpanded] = useState(false);
  const [models, setModels] = useState<CursorModel[]>([]);
  const [defaults, setDefaults] = useState<PipelineModelConfig>({});
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (!expanded || models.length > 0) return;
    setLoading(true);
    listModels()
      .then((res) => {
        setModels(res.models);
        setDefaults(res.defaults);
        setLoadError(null);
      })
      .catch((e) => {
        setLoadError(e instanceof Error ? e.message : '모델 목록 로드 실패');
      })
      .finally(() => setLoading(false));
  }, [expanded, models.length]);

  const setField = (key: keyof PipelineModelConfig, modelId: string) => {
    onChange({
      ...value,
      [key]: modelId || undefined,
    });
  };

  const resetDefaults = () => onChange({});

  const resolved = (key: keyof PipelineModelConfig) =>
    value[key] ?? defaults[key] ?? '';

  return (
    <section className="mb-6 rounded-xl border border-border bg-bg-elevated p-5">
      <button
        type="button"
        className="flex w-full items-center justify-between text-left"
        onClick={() => setExpanded((v) => !v)}
      >
        <div>
          <h3 className="flex items-center gap-2 font-headline text-base font-semibold">
            <span className="material-symbols-outlined text-lg text-accent">tune</span>
            AI 모델 설정
          </h3>
          <p className="mt-1 text-sm text-muted">
            스테이지별 모델을 선택하세요. 비워두면 기본값이 사용됩니다.
          </p>
        </div>
        <span className="material-symbols-outlined text-muted">
          {expanded ? 'expand_less' : 'expand_more'}
        </span>
      </button>

      {expanded && (
        <div className="mt-5 space-y-4 border-t border-border/50 pt-5">
          {loading && <p className="text-sm text-muted">모델 목록 불러오는 중...</p>}
          {loadError && (
            <p className="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-sm text-error">
              {loadError}
            </p>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            {STAGE_FIELDS.map(({ key, label, description }) => (
              <label key={key} className="block text-sm">
                <span className="font-medium text-foreground">{label}</span>
                <span className="mb-1.5 block text-xs text-muted">{description}</span>
                <select
                  value={value[key] ?? ''}
                  onChange={(e) => setField(key, e.target.value)}
                  className="w-full rounded-lg border border-border bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-accent"
                >
                  <option value="">
                    기본값 ({resolved(key) || '…'})
                  </option>
                  {models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name ? `${m.name} (${m.id})` : m.id}
                    </option>
                  ))}
                </select>
              </label>
            ))}
          </div>

          <label className="block text-sm sm:max-w-xs">
            <span className="font-medium text-foreground">Design (Stitch)</span>
            <span className="mb-1.5 block text-xs text-muted">UI 디자인 디바이스 타입</span>
            <select
              value={value.design_device_type ?? 'DESKTOP'}
              onChange={(e) =>
                onChange({ ...value, design_device_type: e.target.value })
              }
              className="w-full rounded-lg border border-border bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-accent"
            >
              <option value="DESKTOP">Desktop</option>
              <option value="MOBILE">Mobile</option>
            </select>
          </label>

          <button
            type="button"
            onClick={resetDefaults}
            className="text-sm text-muted transition-colors hover:text-primary"
          >
            모두 기본값으로 초기화
          </button>
        </div>
      )}
    </section>
  );
}

export function resolveStageModel(
  stage: StageId,
  config?: PipelineModelConfig,
  defaults?: PipelineModelConfig,
): string | undefined {
  if (stage === 'design') {
    const device = config?.design_device_type ?? defaults?.design_device_type ?? 'DESKTOP';
    return `Stitch (${device})`;
  }
  if (stage === 'ingest' || stage === 'deliver') return undefined;

  const key = stage as keyof PipelineModelConfig;
  return (config?.[key] as string | undefined) ?? (defaults?.[key] as string | undefined);
}
