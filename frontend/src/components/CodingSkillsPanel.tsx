import { useEffect, useState } from 'react';
import { getCodingSkills, saveCodingSkills } from '../api/client';
import type { CodingSkill, CodingSkills } from '../types';

const choices: readonly { id: CodingSkill; title: string; description: string }[] = [
  { id: 'targeted_read', title: '필요한 코드만 읽기', description: '심볼 검색 후 관련 파일·범위만 읽고 반복 조회를 줄입니다.' },
  { id: 'minimal_diff', title: '최소 변경', description: '기존 코드·의존성을 재사용하고 요청 범위의 변경과 검증에 집중합니다.' },
  { id: 'concise_output', title: '출력 줄이기', description: '코드와 전체 로그를 반복 출력하지 않고 결과·오류를 요약합니다.' },
];

export function CodingSkillsPanel({ value, onChange }: { value: CodingSkills; onChange: (value: CodingSkills) => void }) {
  const [fileError, setFileError] = useState('');
  return (
    <fieldset className="skill-fields mb-5 min-w-0 rounded-lg border border-border p-5">
      <legend className="px-2 font-medium">코딩 스킬 · 토큰 절약</legend>
      <div className="grid gap-3">
        {choices.map((choice) => (
          <label key={choice.id} className="flex min-h-11 cursor-pointer items-start gap-3">
            <input type="checkbox" className="mt-1 size-5 shrink-0" checked={value.enabled.includes(choice.id)} onChange={(event) => onChange({
              ...value, enabled: event.target.checked ? [...value.enabled, choice.id] : value.enabled.filter((id) => id !== choice.id),
            })} />
            <span className="min-w-0"><strong className="block text-sm">{choice.title}</strong><span className="text-sm text-muted">{choice.description}</span></span>
          </label>
        ))}
        <label className="grid gap-2 text-sm">사용자 스킬 지침 (Markdown, 최대 4,000자)
          <textarea className="workspace-input" rows={4} maxLength={4000} value={value.custom_markdown} onChange={(event) => onChange({ ...value, custom_markdown: event.target.value })} />
        </label>
        <label className="grid gap-2 text-sm">SKILL.md 불러오기
          <input type="file" accept=".md,.markdown,text/markdown,text/plain" onChange={async (event) => {
            const file = event.target.files?.[0];
            event.target.value = '';
            if (!file) return;
            try {
              if (file.size > 16000) { setFileError('스킬 파일은 16KB 이하만 불러올 수 있습니다.'); return; }
              const text = await file.text();
              if ([...text].length > 4000) { setFileError('스킬 지침은 4,000자 이하여야 합니다.'); return; }
              onChange({ ...value, custom_markdown: text });
              setFileError('');
            } catch (error) { setFileError(error instanceof Error ? error.message : '스킬 파일을 읽지 못했습니다.'); }
          }} />
        </label>
        {fileError && <p role="alert" className="text-sm text-error">{fileError}</p>}
        <p className="text-sm text-muted">선택한 지침만 Implement·Verify·Debug·Security 단계에 전달합니다. 파일에 연결된 스크립트나 외부 스킬은 설치하지 않습니다. 절감량은 작업에 따라 다릅니다.</p>
      </div>
    </fieldset>
  );
}

export function ProjectCodingSkills({ projectId }: { projectId: string }) {
  const [value, setValue] = useState<CodingSkills>({ enabled: [], custom_markdown: '' });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true);
    getCodingSkills(projectId).then((skills) => { if (active) { setValue(skills); setError(''); } })
      .catch((err: unknown) => { if (active) setError(err instanceof Error ? err.message : '스킬 조회 실패'); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [projectId, retry]);
  return <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
    {loading ? <p role="status">스킬 설정 불러오는 중...</p> : <>
      <fieldset className="min-w-0" disabled={saving || !!error}>
        <CodingSkillsPanel value={value} onChange={(skills) => { setValue(skills); setMessage(''); }} />
        <button type="button" className="workspace-button" onClick={async () => {
          setSaving(true); setMessage('');
          try { await saveCodingSkills(projectId, value); setMessage('저장했습니다. 다음 코드 작성 단계부터 적용됩니다. 실행 중인 단계에는 반영되지 않습니다.'); }
          catch (err) { setMessage(err instanceof Error ? err.message : '스킬 저장 실패'); }
          finally { setSaving(false); }
        }}>{saving ? '스킬 저장 중...' : '스킬 적용'}</button>
      </fieldset>
      {error && <div role="alert"><p className="text-error">{error}</p><button type="button" className="workspace-button" onClick={() => setRetry((n) => n + 1)}>다시 불러오기</button></div>}
      <p role="status" className="mt-3 text-sm text-muted">{message}</p>
    </>}
  </section>;
}
