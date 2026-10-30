import { useEffect, useRef, useState } from 'react';
import { createPortal, flushSync } from 'react-dom';
import { generateEstimate, getEstimateGeneration, listEstimates, saveEstimate } from '../api/client';
import type { Estimate, EstimateInput, EstimateItem } from '../types';

const won = (amount: number) => `${amount.toLocaleString('ko-KR')}원`;
function emptyDraft(): EstimateInput {
  const today = new Date();
  const valid = new Date(today);
  valid.setDate(valid.getDate() + 30);
  const date = (v: Date) => `${v.getFullYear()}-${String(v.getMonth() + 1).padStart(2, '0')}-${String(v.getDate()).padStart(2, '0')}`;
  return { title: new URLSearchParams(window.location.search).get('title') ?? '', customer: '', supplier: '',
    issued_on: date(today), valid_until: date(valid), items: [{ description: '', quantity: 1, unit_price: 0 }], include_vat: true, notes: '', requirements: '', daily_rate: null };
}

export function EstimatesPage() {
  const [draft, setDraft] = useState<EstimateInput>(emptyDraft);
  const [quotes, setQuotes] = useState<Estimate[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [requirements, setRequirements] = useState('');
  const [dailyRate, setDailyRate] = useState('');
  const [jobId, setJobId] = useState<string | null>(null);
  const [generating, setGenerating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [message, setMessage] = useState('');
  const [dirty, setDirty] = useState(false);
  const [retry, setRetry] = useState(0);
  const [printKind, setPrintKind] = useState<'가견적서' | '견적서'>('가견적서');
  const form = useRef<HTMLFormElement>(null);
  const busy = saving || generating;
  const subtotal = draft.items.reduce((sum, item) => sum + item.quantity * item.unit_price, 0);
  const vat = draft.include_vat ? Math.floor(subtotal / 10) : 0;
  const total = subtotal + vat;
  const invalidTotal = !Number.isSafeInteger(total) || total > 1_000_000_000_000 || total < 0;

  useEffect(() => {
    let active = true;
    setLoading(true);
    listEstimates().then((data) => { if (active) { setQuotes(data); setLoadError(''); } })
      .catch((err: unknown) => { if (active) setLoadError(err instanceof Error ? err.message : '견적 목록 조회 실패'); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [retry]);

  useEffect(() => {
    if (!jobId) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const result = await getEstimateGeneration(jobId);
        if (!active) return;
        switch (result.status) {
          case 'running': timer = setTimeout(() => { void poll(); }, 3000); break;
          case 'completed':
            setDraft((current) => ({ ...current, items: result.draft.items, notes: result.draft.notes }));
            setDirty(true); setJobId(null); setGenerating(false);
            setMessage('AI 가견적을 반영했습니다. 작업 범위·인일·단가를 검토한 뒤 저장해주세요.'); break;
          case 'failed': setError(result.error); setJobId(null); setGenerating(false); setMessage(''); break;
          default: { const unexpected: never = result; throw new Error(`알 수 없는 생성 상태: ${String(unexpected)}`); }
        }
      } catch (err) {
        if (active) { setError(err instanceof Error ? err.message : '가견적 결과 조회 실패'); setJobId(null); setGenerating(false); setMessage(''); }
      }
    };
    void poll();
    return () => { active = false; clearTimeout(timer); };
  }, [jobId]);

  const change = (next: Partial<EstimateInput>) => { setDraft((current) => ({ ...current, ...next })); setDirty(true); setMessage(''); };
  const updateItem = (index: number, patch: Partial<EstimateItem>) => change({ items: draft.items.map((item, i) => i === index ? { ...item, ...patch } : item) });
  const validForm = () => {
    if (!form.current?.reportValidity()) return false;
    if (!draft.title.trim() || draft.items.some((item) => !item.description.trim())) {
      setError('견적 제목과 작업 항목을 입력해주세요.'); return false;
    }
    if (invalidTotal) { setError('합계는 1조 원 이하의 정수여야 합니다.'); return false; }
    return true;
  };
  const print = (kind: typeof printKind) => {
    if (!validForm()) return;
    flushSync(() => setPrintKind(kind));
    const title = document.title;
    document.title = `${draft.title} ${kind}`;
    window.print();
    document.title = title;
  };

  return <div className="estimate-page mx-auto w-full max-w-5xl px-5 py-8 md:px-8">
    <h2 className="mb-6 text-2xl font-semibold">견적 작성</h2>
    <div className="mb-6 flex flex-wrap items-end gap-3">
      <label className="grid min-w-0 flex-1 gap-2 text-sm">저장된 견적
        <select className="workspace-input" value={selectedId ?? ''} disabled={loading || busy} onChange={(event) => {
          if (dirty && !window.confirm('저장하지 않은 변경을 버리고 다른 견적을 여시겠습니까?')) return;
          const quote = quotes.find((item) => item.id === event.target.value);
          setSelectedId(quote?.id ?? null); setDraft(quote ?? emptyDraft()); setDirty(false); setError(''); setMessage('');
          setRequirements(quote?.requirements ?? ''); setDailyRate(quote?.daily_rate?.toString() ?? '');
        }}><option value="">{loading ? '목록 불러오는 중...' : '새 견적'}</option>
          {quotes.map((quote) => <option key={quote.id} value={quote.id}>{quote.title} · {quote.customer || '수신처 미입력'} · {quote.issued_on}</option>)}
        </select>
      </label>
      <button type="button" className="workspace-button" disabled={busy} onClick={() => {
        if (dirty && !window.confirm('저장하지 않은 변경을 버리고 새 견적을 작성하시겠습니까?')) return;
        setSelectedId(null); setDraft(emptyDraft()); setDirty(false); setMessage(''); setError('');
        setRequirements(''); setDailyRate('');
      }}>새 견적</button>
    </div>
    {loadError && <div role="alert" className="mb-4 text-error">{loadError} <button className="workspace-button" onClick={() => setRetry((n) => n + 1)}>목록 다시 불러오기</button></div>}
    <form ref={form} onSubmit={async (event) => {
      event.preventDefault(); if (!validForm()) return;
      setSaving(true); setError(''); setMessage('');
      try {
        const quote = await saveEstimate({ ...draft, requirements, daily_rate: dailyRate ? Number(dailyRate) : null }, selectedId);
        setSelectedId(quote.id); setDraft(quote); setQuotes((current) => [quote, ...current.filter((item) => item.id !== quote.id)]);
        setDirty(false); setMessage('견적을 저장했습니다.');
      } catch (err) { setError(err instanceof Error ? err.message : '견적 저장 실패'); }
      finally { setSaving(false); }
    }}>
      <fieldset disabled={busy} className="grid gap-6">
        <section className="rounded-lg border border-border bg-card p-5">
          <h3 className="mb-4 font-medium">견적 정보</h3>
          <div className="grid gap-4 md:grid-cols-2">
            <label className="grid gap-2 text-sm md:col-span-2">프로젝트 / 견적 제목<input className="workspace-input" required maxLength={200} value={draft.title} onChange={(e) => change({ title: e.target.value })} /></label>
            <label className="grid gap-2 text-sm">수신처<input className="workspace-input" maxLength={200} value={draft.customer} onChange={(e) => change({ customer: e.target.value })} /></label>
            <label className="grid gap-2 text-sm">공급자 / 연락처<input className="workspace-input" maxLength={300} value={draft.supplier} onChange={(e) => change({ supplier: e.target.value })} /></label>
            <label className="grid gap-2 text-sm">발행일<input className="workspace-input" type="date" required value={draft.issued_on} onChange={(e) => change({ issued_on: e.target.value })} /></label>
            <label className="grid gap-2 text-sm">유효기한<input className="workspace-input" type="date" required min={draft.issued_on} value={draft.valid_until} onChange={(e) => change({ valid_until: e.target.value })} /></label>
          </div>
        </section>
        <section className="rounded-lg border border-border bg-card p-5">
          <h3 className="mb-4 font-medium">요구사항으로 AI 가견적 받기</h3>
          <label className="mb-4 grid gap-2 text-sm">요구사항<textarea className="workspace-input" rows={5} maxLength={6000} value={requirements} placeholder="필요한 기능, 화면, 외부 연동, 배포 환경과 제외 범위를 적어주세요." onChange={(e) => { setRequirements(e.target.value); setDirty(true); }} /></label>
          <div className="flex flex-wrap items-end gap-3">
            <label className="grid gap-2 text-sm">1일 단가 (원)<input className="workspace-input" type="number" min={1} max={1000000000} step={1} value={dailyRate} onChange={(e) => { setDailyRate(e.target.value); setDirty(true); }} /></label>
            <button type="button" className="workspace-button" onClick={async () => {
              const rate = Number(dailyRate);
              if (!draft.title.trim() || [...requirements.trim()].length < 10 || !Number.isInteger(rate) || rate <= 0 || rate > 1_000_000_000) { setError('제목, 요구사항(10자 이상), 1일 단가를 입력해주세요.'); return; }
              if ((draft.items.some((item) => item.description.trim()) || draft.notes.trim()) && !window.confirm('현재 작업 항목과 비고를 AI 가견적으로 교체하시겠습니까?')) return;
              setGenerating(true); setError(''); setMessage('AI가 작업별 인일을 산정하고 있습니다. 최대 15분이 걸릴 수 있습니다.');
              try { const job = await generateEstimate({ requirements, daily_rate: rate, title: draft.title }); setJobId(job.id); }
              catch (err) { setError(err instanceof Error ? err.message : 'AI 가견적 요청 실패'); setGenerating(false); setMessage(''); }
            }}>AI 가견적 생성</button>
          </div>
          <p className="mt-3 text-sm text-muted">AI 예상 인일 × 입력한 단가로 계산합니다. 생성 결과는 아래 작업 항목·비고를 교체하며, 금액은 검토 후 수정할 수 있습니다.</p>
        </section>
        <section className="rounded-lg border border-border bg-card p-5">
          <h3 className="mb-4 font-medium">작업 항목</h3>
          <div className="grid gap-4">
            {draft.items.map((item, index) => <div key={index} className="estimate-item grid gap-3 border-b border-border pb-4">
              <label className="grid min-w-0 gap-2 text-sm">항목 {index + 1}<input className="workspace-input" required maxLength={500} value={item.description} onChange={(e) => updateItem(index, { description: e.target.value })} /></label>
              <label className="grid min-w-0 gap-2 text-sm">수량 / 인일<input className="workspace-input" type="number" required min={1} max={10000} step={1} value={Number.isNaN(item.quantity) ? '' : item.quantity} onChange={(e) => updateItem(index, { quantity: e.target.valueAsNumber })} /></label>
              <label className="grid min-w-0 gap-2 text-sm">단가 (원)<input className="workspace-input" type="number" required min={0} max={1000000000} step={1} value={Number.isNaN(item.unit_price) ? '' : item.unit_price} onChange={(e) => updateItem(index, { unit_price: e.target.valueAsNumber })} /></label>
              <div className="flex items-end justify-between gap-3"><output className="pb-3 text-sm">{Number.isFinite(item.quantity * item.unit_price) ? won(item.quantity * item.unit_price) : '-'}</output><button type="button" className="workspace-button" aria-label={`항목 ${index + 1} 삭제`} disabled={draft.items.length === 1} onClick={() => change({ items: draft.items.filter((_, i) => i !== index) })}>삭제</button></div>
            </div>)}
          </div>
          <button type="button" className="workspace-button mt-4" disabled={draft.items.length >= 100} onClick={() => change({ items: [...draft.items, { description: '', quantity: 1, unit_price: 0 }] })}>항목 추가</button>
          <label className="my-5 flex min-h-11 items-center gap-3 text-sm"><input type="checkbox" className="size-5" checked={draft.include_vat} onChange={(e) => change({ include_vat: e.target.checked })} />부가세 10% 별도 가산 (원 미만 버림)</label>
          <dl className="ml-auto grid max-w-sm grid-cols-2 gap-3 text-sm"><dt>공급가액</dt><dd className="text-right">{invalidTotal ? '-' : won(subtotal)}</dd><dt>부가세</dt><dd className="text-right">{invalidTotal ? '-' : won(vat)}</dd><dt className="text-lg font-semibold">합계</dt><dd className="text-right text-lg font-semibold">{invalidTotal ? '-' : won(total)}</dd></dl>
        </section>
        <label className="grid gap-2 text-sm">비고 / 가정 / 제외사항<textarea className="workspace-input" rows={5} maxLength={4000} value={draft.notes} onChange={(e) => change({ notes: e.target.value })} /></label>
        <div className="flex flex-wrap gap-3">
          <button type="submit" className="workspace-button workspace-primary">{saving ? '견적 저장 중...' : '견적 저장'}</button>
          <button type="button" className="workspace-button" onClick={() => print('가견적서')}>가견적서 PDF</button>
          <button type="button" className="workspace-button" onClick={() => print('견적서')}>견적서 PDF</button>
        </div>
      </fieldset>
    </form>
    <p className="mt-4 text-sm text-muted">PDF 버튼을 누른 뒤 인쇄 창에서 ‘PDF로 저장’을 선택하세요. 현재 입력한 내용으로 출력하며, 저장 전 수정 사항도 포함합니다.</p>
    {dirty && <p className="mt-3 text-sm text-warn">저장하지 않은 변경이 있습니다.</p>}
    {error && <p role="alert" className="mt-4 text-sm text-error">{error}</p>}
    <p role="status" className="mt-4 text-sm">{message}</p>
    {createPortal(<article className="estimate-print">
      <header><h1>{printKind}</h1><h2>{draft.title}</h2><p>발행일 {draft.issued_on} · 유효기한 {draft.valid_until}</p></header>
      <dl className="estimate-parties"><dt>수신처</dt><dd>{draft.customer || '-'}</dd><dt>공급자</dt><dd>{draft.supplier || '-'}</dd></dl>
      {printKind === '가견적서' && <p className="estimate-notice">본 가견적은 협의용 예상 금액이며, 최종 범위 및 조건에 따라 변경될 수 있습니다.</p>}
      <table><colgroup><col style={{ width: '46%' }} /><col style={{ width: '14%' }} /><col style={{ width: '20%' }} /><col style={{ width: '20%' }} /></colgroup><thead><tr><th>작업 항목</th><th>수량 / 인일</th><th>단가 (원)</th><th>금액 (원)</th></tr></thead><tbody>{draft.items.map((item, index) => <tr key={index}><td>{item.description}</td><td>{item.quantity}</td><td>{item.unit_price.toLocaleString('ko-KR')}</td><td>{(item.quantity * item.unit_price).toLocaleString('ko-KR')}</td></tr>)}</tbody></table>
      <section className="estimate-totals"><p>공급가액 <strong>{won(subtotal)}</strong></p><p>부가세 {draft.include_vat ? '(10%)' : '(미가산)'} <strong>{won(vat)}</strong></p><p>합계 <strong>{won(total)}</strong></p></section>
      {draft.notes && <section className="estimate-notes"><h3>비고 / 가정 / 제외사항</h3><p>{draft.notes}</p></section>}
    </article>, document.body)}
  </div>;
}
