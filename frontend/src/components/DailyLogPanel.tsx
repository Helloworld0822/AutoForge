import { useEffect, useState } from 'react';
import { CalendarDays, Loader2 } from 'lucide-react';
import { getDailyLog, listDailyLogs } from '../api/client';
import type { DailyLog, DailyLogSummary } from '../types';

function MarkdownBody({ content }: { content: string }) {
  return (
    <pre className="m-0 whitespace-pre-wrap break-words font-mono text-[0.78rem] leading-relaxed text-foreground">
      {content}
    </pre>
  );
}

export function DailyLogPanel({ projectId }: { projectId: string }) {
  const [logs, setLogs] = useState<DailyLogSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<DailyLog | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);

  useEffect(() => {
    listDailyLogs(projectId)
      .then((data) => {
        setLogs(data);
        if (data.length > 0) {
          setSelected(data[data.length - 1].date);
        }
      })
      .catch(() => setLogs([]))
      .finally(() => setLoading(false));
  }, [projectId]);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    setDetailLoading(true);
    getDailyLog(projectId, selected)
      .then(setDetail)
      .catch(() => setDetail(null))
      .finally(() => setDetailLoading(false));
  }, [projectId, selected]);

  if (loading) {
    return (
      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6" id="daily-logs">
        <h3 className="mb-4 flex items-center gap-2 text-base font-medium">
          <CalendarDays size={18} /> 일별 경과
        </h3>
        <p className="text-muted">
          <Loader2 size={16} className="inline animate-spin" /> 로딩 중...
        </p>
      </section>
    );
  }

  if (logs.length === 0) {
    return (
      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6" id="daily-logs">
        <h3 className="mb-4 flex items-center gap-2 text-base font-medium">
          <CalendarDays size={18} /> 일별 경과
        </h3>
        <p className="text-muted">아직 기록된 일일 경과가 없습니다.</p>
      </section>
    );
  }

  return (
    <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6" id="daily-logs">
      <h3 className="mb-2 flex items-center gap-2 text-base font-medium">
        <CalendarDays size={18} /> 일별 경과
      </h3>
      <p className="mb-4 text-sm text-muted">
        파이프라인 진행 상황이 날짜별 Markdown으로 정리됩니다. Slack에도 동일
        내용이 전송됩니다.
      </p>

      <div className="grid min-h-[280px] grid-cols-1 gap-4 md:grid-cols-[220px_1fr]">
        <div className="flex flex-col gap-2">
          {logs.map((log) => {
            const active = selected === log.date;
            return (
              <button
                key={log.date}
                type="button"
                id={`daily-${log.date}`}
                className={`flex cursor-pointer flex-col items-start gap-0.5 rounded-lg border px-3 py-2.5 text-left text-foreground transition-colors ${
                  active
                    ? 'border-accent bg-accent-dim'
                    : 'border-border bg-bg hover:border-accent hover:bg-accent-dim'
                }`}
                onClick={() => setSelected(log.date)}
              >
                <span className="text-sm font-semibold">Day {log.day_number}</span>
                <span className="text-xs text-muted">{log.date}</span>
                <span className="text-[0.7rem] text-accent">
                  {log.progress_percent}% · {log.entry_count}건
                </span>
              </button>
            );
          })}
        </div>

        <div className="max-h-[480px] overflow-auto rounded-lg border border-border bg-bg p-4">
          {detailLoading ? (
            <p className="text-muted">
              <Loader2 size={16} className="inline animate-spin" /> 불러오는 중...
            </p>
          ) : detail ? (
            <MarkdownBody content={detail.markdown} />
          ) : (
            <p className="text-muted">일일 경과를 불러올 수 없습니다.</p>
          )}
        </div>
      </div>
    </section>
  );
}
