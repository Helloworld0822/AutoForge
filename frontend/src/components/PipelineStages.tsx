import { resolveStageModel } from './ModelConfigPanel';
import { STAGE_META, type PipelineModelConfig, type StageId, type StageState } from '../types';
import {
  CheckCircle2,
  Circle,
  Loader2,
  MinusCircle,
  XCircle,
} from 'lucide-react';

const statusIcon: Record<StageState, React.ReactNode> = {
  queued: <Circle size={16} className="text-muted" />,
  running: <Loader2 size={16} className="animate-spin text-accent" />,
  completed: <CheckCircle2 size={16} className="text-success" />,
  failed: <XCircle size={16} className="text-error" />,
  skipped: <MinusCircle size={16} className="text-muted" />,
};

const stageCardClass: Record<StageState, string> = {
  queued: 'border-border',
  running: 'border-accent shadow-[0_0_0_1px_var(--color-accent-dim)]',
  completed: 'border-success/40',
  failed: 'border-error/40',
  skipped: 'border-border',
};

const stageStatusClass: Record<StageState, string> = {
  queued: 'text-muted',
  running: 'text-accent',
  completed: 'text-success',
  failed: 'text-error',
  skipped: 'text-muted',
};

export function PipelineStages({
  stages,
  modelConfig,
}: {
  stages: { stage: StageId; status: StageState }[];
  modelConfig?: PipelineModelConfig;
}) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3">
      {stages.map((s, idx) => {
        const meta = STAGE_META[s.stage];
        const model =
          resolveStageModel(s.stage, modelConfig) ?? meta.model;
        return (
          <div
            key={s.stage}
            className={`relative rounded-[10px] border bg-bg p-3.5 ${stageCardClass[s.status]}`}
          >
            <div className="mb-2 flex justify-between">
              {statusIcon[s.status]}
              <span className="text-[0.7rem] text-muted">{idx + 1}</span>
            </div>
            <h4 className="mb-1 text-[0.85rem] font-medium">{meta.label}</h4>
            <p className="text-[0.7rem] leading-snug text-muted">{meta.description}</p>
            {model && (
              <span
                className="mt-2 inline-block max-w-full truncate rounded bg-accent-dim px-1.5 py-0.5 text-[0.65rem] text-accent"
                title={model}
              >
                {model}
              </span>
            )}
            <span
              className={`mt-2 block text-[0.65rem] uppercase tracking-wide ${stageStatusClass[s.status]}`}
            >
              {s.status}
            </span>
          </div>
        );
      })}
    </div>
  );
}
