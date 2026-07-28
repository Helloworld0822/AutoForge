import { Activity, Loader2 } from 'lucide-react';
import type { ActivityEntry, PipelineState } from '../types';
import { STAGE_META } from '../types';

const eventLabel: Record<string, string> = {
  project_created: '프로젝트 생성',
  stage_running: '스테이지 실행',
  stage_completed: '스테이지 완료',
  stage_failed: '스테이지 실패',
  pipeline_failed: '파이프라인 실패',
  pipeline_completed: '파이프라인 완료',
  pipeline_restarted: '재시작',
  architecture_input_required: '입력 필요',
};

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString('ko-KR', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  } catch {
    return iso;
  }
}

function entryTone(event: string): string {
  if (event.includes('failed')) return 'border-error/30 bg-error/5';
  if (event.includes('completed')) return 'border-success/30 bg-success/5';
  if (event === 'stage_running') return 'border-accent/30 bg-accent/5';
  return 'border-border bg-bg';
}

export function PipelineActivityPanel({
  activity,
  state,
  currentStage,
  loading,
}: {
  activity: ActivityEntry[];
  state: PipelineState;
  currentStage?: string | null;
  loading?: boolean;
}) {
  const isActive = state === 'running' || state === 'awaiting_input';

  return (
    <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-base font-medium">
          <Activity size={18} />
          실시간 진행 로그
        </h3>
        {isActive && (
          <span className="inline-flex items-center gap-1.5 text-xs text-accent">
            <Loader2 size={14} className="animate-spin" />
            {currentStage
              ? `${STAGE_META[currentStage as keyof typeof STAGE_META]?.label ?? currentStage} 실행 중`
              : '업데이트 중'}
          </span>
        )}
      </div>

      {loading && activity.length === 0 ? (
        <p className="text-sm text-muted">
          <Loader2 size={16} className="mr-1 inline animate-spin" />
          로그 불러오는 중...
        </p>
      ) : activity.length === 0 ? (
        <p className="text-sm text-muted">아직 기록된 활동이 없습니다.</p>
      ) : (
        <ol className="max-h-[320px] space-y-2 overflow-y-auto pr-1">
          {[...activity].reverse().map((entry, idx) => (
            <li
              key={`${entry.at}-${entry.event}-${idx}`}
              className={`rounded-lg border px-3 py-2.5 text-sm ${entryTone(entry.event)}`}
            >
              <div className="mb-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                <time dateTime={entry.at}>{formatTime(entry.at)}</time>
                <span className="rounded bg-bg px-1.5 py-0.5">
                  {eventLabel[entry.event] ?? entry.event}
                </span>
                {entry.stage && (
                  <span className="text-accent">
                    {STAGE_META[entry.stage]?.label ?? entry.stage}
                  </span>
                )}
                <span>{entry.progress_percent}%</span>
              </div>
              <p className="leading-relaxed text-foreground">{entry.message}</p>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
