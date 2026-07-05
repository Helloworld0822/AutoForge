import { useState } from 'react';
import { Loader2, MessageSquare, Send } from 'lucide-react';
import { submitArchitectureAnswers } from '../api/client';
import type { ArchitectureClarification } from '../types';

interface ArchitectureQnAPanelProps {
  projectId: string;
  clarifications: ArchitectureClarification[];
  onSubmitted: () => void;
}

export function ArchitectureQnAPanel({
  projectId,
  clarifications,
  onSubmitted,
}: ArchitectureQnAPanelProps) {
  const [answers, setAnswers] = useState<Record<string, string>>(() => {
    const initial: Record<string, string> = {};
    for (const q of clarifications) {
      if (q.answer) initial[q.id] = q.answer;
    }
    return initial;
  });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const unansweredRequired = clarifications.filter(
    (q) => q.required && !answers[q.id]?.trim(),
  );

  const handleSubmit = async () => {
    if (unansweredRequired.length > 0) {
      setError('필수 질문에 모두 답변해주세요.');
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      await submitArchitectureAnswers(
        projectId,
        clarifications.map((q) => ({
          id: q.id,
          answer: answers[q.id]?.trim() ?? '',
        })),
      );
      onSubmitted();
    } catch (err) {
      setError(err instanceof Error ? err.message : '답변 제출 실패');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <section className="mb-5 rounded-lg border border-accent/40 bg-accent/5 p-5 md:p-6">
      <h3 className="mb-2 flex items-center gap-2 text-base font-medium text-accent">
        <MessageSquare size={18} />
        아키텍처 설계 질문
      </h3>
      <p className="mb-4 text-sm text-muted">
        AI가 계획서를 분석한 결과, 아래 질문에 답변해주시면 더 정확한 아키텍처를 설계합니다.
      </p>

      <div className="grid gap-4">
        {clarifications.map((q) => (
          <div
            key={q.id}
            className="rounded-lg border border-border bg-bg p-4"
          >
            <div className="mb-2 flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium">{q.question}</span>
              {q.required && (
                <span className="rounded-full bg-error/10 px-2 py-0.5 text-xs text-error">
                  필수
                </span>
              )}
              {q.category && (
                <span className="rounded-full bg-card px-2 py-0.5 text-xs text-muted">
                  {q.category}
                </span>
              )}
            </div>

            {q.options.length > 0 ? (
              <div className="flex flex-wrap gap-2">
                {q.options.map((opt) => {
                  const selected = answers[q.id] === opt;
                  return (
                    <button
                      key={opt}
                      type="button"
                      className={`rounded-lg border px-3 py-1.5 text-sm transition-colors ${
                        selected
                          ? 'border-accent bg-accent-dim text-foreground'
                          : 'border-border bg-card text-muted hover:border-accent'
                      }`}
                      onClick={() =>
                        setAnswers((prev) => ({ ...prev, [q.id]: opt }))
                      }
                    >
                      {opt}
                    </button>
                  );
                })}
              </div>
            ) : (
              <textarea
                rows={3}
                value={answers[q.id] ?? ''}
                onChange={(e) =>
                  setAnswers((prev) => ({ ...prev, [q.id]: e.target.value }))
                }
                placeholder="답변을 입력하세요..."
                className="mt-1 block w-full resize-y rounded-lg border border-border bg-card px-3 py-2 text-sm text-foreground"
              />
            )}
          </div>
        ))}
      </div>

      {error && (
        <div className="mt-4 rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
          {error}
        </div>
      )}

      <button
        type="button"
        className="mt-4 inline-flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3 font-medium text-white transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50 sm:w-auto"
        onClick={handleSubmit}
        disabled={submitting || unansweredRequired.length > 0}
      >
        {submitting ? (
          <>
            <Loader2 size={18} className="animate-spin" />
            제출 중...
          </>
        ) : (
          <>
            <Send size={18} />
            답변 제출하고 설계 계속하기
          </>
        )}
      </button>
    </section>
  );
}
