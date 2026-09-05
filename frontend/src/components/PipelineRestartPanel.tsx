import { useState } from 'react';
import { restartProject } from '../api/client';
import { ModelConfigPanel } from './ModelConfigPanel';
import { STAGE_META, type PipelineModelConfig, type StageId } from '../types';
import { Loader2, RotateCcw } from 'lucide-react';

interface PipelineRestartPanelProps {
  projectId: string;
  failedStage?: StageId;
  cancelled?: boolean;
  initialModelConfig?: PipelineModelConfig;
  onRestarted: () => void;
}

export function PipelineRestartPanel({
  projectId,
  failedStage,
  cancelled = false,
  initialModelConfig,
  onRestarted,
}: PipelineRestartPanelProps) {
  const [modelConfig, setModelConfig] = useState<PipelineModelConfig>(
    initialModelConfig ?? {},
  );
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const failedLabel = failedStage ? STAGE_META[failedStage].label : null;

  const handleRestart = async () => {
    setRestarting(true);
    setError(null);
    try {
      await restartProject(projectId, {
        modelConfig,
        fromStage: failedStage,
      });
      onRestarted();
    } catch (err) {
      setError(err instanceof Error ? err.message : '재시작 실패');
    } finally {
      setRestarting(false);
    }
  };

  return (
    <section className="mb-5 rounded-lg border border-error/30 bg-error/5 p-5 md:p-6">
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="flex items-center gap-2 text-base font-medium text-error">
            <RotateCcw size={18} />
            파이프라인 재시작
          </h3>
          <p className="mt-1 text-sm text-muted">
            {cancelled
              ? '취소된 파이프라인입니다. AI 모델을 변경한 뒤 다시 시작할 수 있습니다.'
              : failedLabel
              ? `${failedLabel} 단계에서 실패했습니다. AI 모델을 변경한 뒤 재시작할 수 있습니다.`
              : 'AI 모델을 변경한 뒤 파이프라인을 재시작할 수 있습니다.'}
          </p>
        </div>
        <button
          type="button"
          onClick={handleRestart}
          disabled={restarting}
          className="inline-flex items-center justify-center gap-2 rounded-lg bg-accent px-4 py-2.5 font-medium text-white transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {restarting ? (
            <>
              <Loader2 size={16} className="animate-spin" />
              재시작 중…
            </>
          ) : (
            <>
              <RotateCcw size={16} />
              재시작
            </>
          )}
        </button>
      </div>

      {error && (
        <p className="mb-4 rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-sm text-error">
          {error}
        </p>
      )}

      <ModelConfigPanel
        value={modelConfig}
        onChange={setModelConfig}
        defaultExpanded
      />
    </section>
  );

}
