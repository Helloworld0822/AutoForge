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
    stage: 'extract',
    label: 'Extract',
    description: 'PDF 요구사항 구조화 추출',
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
  defaultExpanded?: boolean;
}

export function ModelConfigPanel({ value, onChange, defaultExpanded = false }: ModelConfigPanelProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [models, setModels] = useState<CursorModel[]>([]);
  const [omnirouteModels, setOmnirouteModels] = useState<readonly CursorModel[]>([]);
  const [defaults, setDefaults] = useState<PipelineModelConfig>({});
  const [catalogLoaded, setCatalogLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (!expanded || catalogLoaded) return;
    setLoading(true);
    listModels()
      .then((res) => {
        setModels(res.models);
        setOmnirouteModels(res.omniroute_models);
        setDefaults(res.defaults);
        setCatalogLoaded(true);
        setLoadError(null);
      })
      .catch((e) => {
        setLoadError(e instanceof Error ? e.message : '모델 목록 로드 실패');
      })
      .finally(() => setLoading(false));
  }, [catalogLoaded, expanded]);

  const setField = (key: keyof PipelineModelConfig, modelId: string) => {
    onChange({
      ...value,
      [key]: modelId || undefined,
    });
  };

  const resetDefaults = () => onChange({});

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
            Extract·Architect는 OmniRoute, 구현·품질 단계는 Cursor executor 모델을 사용합니다.
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
            {STAGE_FIELDS.map(({ key, label, description }) => {
              const availableModels =
                key === 'summarize' || key === 'architect' ? omnirouteModels : models;
              const configuredModel = value[key] ?? '';
              const hasUnlistedConfiguredModel =
                configuredModel !== '' &&
                !availableModels.some((model) => model.id === configuredModel);
              const missingFromLoadedCatalog =
                hasUnlistedConfiguredModel && catalogLoaded && !loadError;
              const warningId = `${String(key)}-model-warning`;
              return (
                <label key={key} className="block text-sm">
                  <span className="font-medium text-foreground">{label}</span>
                  <span className="mb-1.5 block text-xs text-muted">{description}</span>
                  <select
                    value={configuredModel}
                    onChange={(e) => setField(key, e.target.value)}
                    aria-describedby={missingFromLoadedCatalog ? warningId : undefined}
                    className="w-full rounded-lg border border-border bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-accent"
                  >
                    <option value="">기본값 ({defaults[key] || (key === 'summarize' || key === 'architect' ? 'OmniRoute 역할 설정 필요' : 'Cursor 계정 기본 모델')})</option>
                    {hasUnlistedConfiguredModel && (
                      <option value={configuredModel}>
                        현재 설정: {configuredModel}
                        {missingFromLoadedCatalog ? ' (카탈로그에 없음)' : ''}
                      </option>
                    )}
                    {availableModels.map((model) => (
                      <option key={model.id} value={model.id}>
                        {model.name ? `${model.name} (${model.id})` : model.id}
                      </option>
                    ))}
                  </select>
                  {missingFromLoadedCatalog && (
                    <span id={warningId} className="mt-1 block text-xs text-warn">
                      현재 설정된 모델 ID가 카탈로그에 없습니다. 이 ID가 그대로 제출됩니다.
                    </span>
                  )}
                </label>
              );
            })}
          </div>

          <div className="space-y-4 rounded-lg border border-border/60 bg-surface-container-lowest/40 p-4">
            <label className="block text-sm sm:max-w-xs">
              <span className="font-medium text-foreground">Design 소스</span>
              <span className="mb-1.5 block text-xs text-muted">
                Stitch AI 생성 또는 기존 Figma 파일에서 UI 참고 자료 추출
              </span>
              <select
                value={value.design_source ?? 'stitch'}
                onChange={(e) =>
                  onChange({
                    ...value,
                    design_source: e.target.value as 'stitch' | 'figma',
                  })
                }
                className="w-full rounded-lg border border-border bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-accent"
              >
                <option value="stitch">Stitch (AI 생성)</option>
                <option value="figma">Figma (디자인 파일)</option>
              </select>
            </label>

            {(value.design_source ?? 'stitch') === 'stitch' ? (
              <label className="block text-sm sm:max-w-xs">
                <span className="font-medium text-foreground">Stitch 디바이스</span>
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
            ) : (
              <label className="block text-sm">
                <span className="font-medium text-foreground">Figma 파일 URL</span>
                <span className="mb-1.5 block text-xs text-muted">
                  figma.com/design/... 또는 frame URL (node-id 포함 권장)
                </span>
                <input
                  type="url"
                  value={value.figma_file_url ?? ''}
                  onChange={(e) =>
                    onChange({ ...value, figma_file_url: e.target.value || undefined })
                  }
                  placeholder="https://www.figma.com/design/..."
                  className="w-full rounded-lg border border-border bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-accent"
                />
              </label>
            )}
          </div>

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
    const source = config?.design_source ?? defaults?.design_source ?? 'stitch';
    if (source === 'figma') {
      return 'Figma';
    }
    const device = config?.design_device_type ?? defaults?.design_device_type ?? 'DESKTOP';
    return `Stitch (${device})`;
  }
  if (stage === 'ingest' || stage === 'deliver') return undefined;

  const key = (stage === 'extract' ? 'summarize' : stage) as keyof PipelineModelConfig;
  return (config?.[key] as string | undefined) ?? (defaults?.[key] as string | undefined);
}
