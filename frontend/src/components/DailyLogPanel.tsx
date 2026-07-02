import { useEffect, useState } from 'react';
import { CalendarDays, Loader2 } from 'lucide-react';
import { getDailyLog, listDailyLogs } from '../api/client';
import type { DailyLog, DailyLogSummary } from '../types';

function MarkdownBody({ content }: { content: string }) {
  return <pre className="markdown-body">{content}</pre>;
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
      <section className="card" id="daily-logs">
        <h3>
          <CalendarDays size={18} /> 일별 경과
        </h3>
        <p className="muted">
          <Loader2 size={16} className="spin" /> 로딩 중...
        </p>
      </section>
    );
  }

  if (logs.length === 0) {
    return (
      <section className="card" id="daily-logs">
        <h3>
          <CalendarDays size={18} /> 일별 경과
        </h3>
        <p className="muted">아직 기록된 일일 경과가 없습니다.</p>
      </section>
    );
  }

  return (
    <section className="card" id="daily-logs">
      <h3>
        <CalendarDays size={18} /> 일별 경과
      </h3>
      <p className="section-desc">
        파이프라인 진행 상황이 날짜별 Markdown으로 정리됩니다. Slack에도 동일
        내용이 전송됩니다.
      </p>

      <div className="daily-log-layout">
        <div className="daily-log-tabs">
          {logs.map((log) => (
            <button
              key={log.date}
              type="button"
              id={`daily-${log.date}`}
              className={
                selected === log.date ? 'daily-tab active' : 'daily-tab'
              }
              onClick={() => setSelected(log.date)}
            >
              <span className="daily-tab-day">Day {log.day_number}</span>
              <span className="daily-tab-date">{log.date}</span>
              <span className="daily-tab-meta">
                {log.progress_percent}% · {log.entry_count}건
              </span>
            </button>
          ))}
        </div>

        <div className="daily-log-content">
          {detailLoading ? (
            <p className="muted">
              <Loader2 size={16} className="spin" /> 불러오는 중...
            </p>
          ) : detail ? (
            <MarkdownBody content={detail.markdown} />
          ) : (
            <p className="muted">일일 경과를 불러올 수 없습니다.</p>
          )}
        </div>
      </div>
    </section>
  );
}
