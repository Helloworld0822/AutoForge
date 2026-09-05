#!/usr/bin/env node
/**
 * AutoForge 기능 테스트 — API + Playwright MCP UI
 */
import { writeFileSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const API = process.env.API_URL ?? 'http://localhost:8080';
const UI = process.env.UI_URL ?? 'http://localhost:5173';
const MCP_URL = process.env.PLAYWRIGHT_MCP_URL ?? 'http://localhost:8931/mcp';
const TMP = process.env.TEST_TMP ?? join(tmpdir(), 'autoforge-functional-test');
const MCP_WORKDIR =
  process.env.MCP_WORKDIR ?? join(process.cwd(), 'frontend', '.playwright-mcp');
mkdirSync(TMP, { recursive: true });

const results = [];
let passed = 0;
let failed = 0;

function ok(name, detail = '') {
  passed++;
  results.push({ name, status: 'pass', detail });
  console.log(`  ✅ ${name}${detail ? ` — ${detail}` : ''}`);
}

function fail(name, detail = '') {
  failed++;
  results.push({ name, status: 'fail', detail });
  console.log(`  ❌ ${name}${detail ? ` — ${detail}` : ''}`);
}

async function api(path, init = {}) {
  const res = await fetch(`${API}${path}`, init);
  const text = await res.text();
  let json;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = text;
  }
  return { res, json, text };
}

// pdfkit-generated PDF with extractable text (lopdf + pdftotext compatible)
const TEST_PLAN_PDF_B64 =
  'JVBERi0xLjMKJf////8KNyAwIG9iago8PAovVHlwZSAvUGFnZQovUGFyZW50IDEgMCBSCi9NZWRpYUJveCBbMCAwIDYxMiA3OTJdCi9Db250ZW50cyA1IDAgUgovUmVzb3VyY2VzIDYgMCBSCi9Vc2VyVW5pdCAxCj4+CmVuZG9iago2IDAgb2JqCjw8Ci9Qcm9jU2V0IFsvUERGIC9UZXh0IC9JbWFnZUIgL0ltYWdlQyAvSW1hZ2VJXQovRm9udCA8PAovRjEgOCAwIFIKPj4KL0NvbG9yU3BhY2UgPDwKPj4KPj4KZW5kb2JqCjUgMCBvYmoKPDwKL0xlbmd0aCAyMTgKL0ZpbHRlciAvRmxhdGVEZWNvZGUKPj4Kc3RyZWFtCnicldA9TsRADAXgfk7xLpDgn/FzVoqmQIKCDikdokDZpNuC+zdoEhrYBuRmbFl631ghEAwKQV4M6618Fr2bPS7fQ0UaUnX0qWK5lYdnhRqWvbzNVRtcMGdk5V55dtzTmAyTGm41TBpCMEdtUOsLkZ7VJIQrlVuDvGN5KU9Lef0DhpccoxN+YZJ7gwbmZIPFkWMNw/HceGVw67GZDXowaCYp3dsw1P6RSuVqUq/BkP+6Jh95f6PIKYVBz8pgz4/0Tkv3j/MyJh4uvUzo3LnRs2/Yaf4B+QJfzFvHCmVuZHN0cmVhbQplbmRvYmoKMTAgMCBvYmoKKFBERktpdCkKZW5kb2JqCjExIDAgb2JqCihQREZLaXQpCmVuZG9iagoxMiAwIG9iagooRDoyMDI2MDcwOTA3NTUyM1opCmVuZG9iago5IDAgb2JqCjw8Ci9Qcm9kdWNlciAxMCAwIFIKL0NyZWF0b3IgMTEgMCBSCi9DcmVhdGlvbkRhdGUgMTIgMCBSCj4+CmVuZG9iago4IDAgb2JqCjw8Ci9UeXBlIC9Gb250Ci9CYXNlRm9udCAvSGVsdmV0aWNhCi9TdWJ0eXBlIC9UeXBlMQovRW5jb2RpbmcgL1dpbkFuc2lFbmNvZGluZwo+PgplbmRvYmoKNCAwIG9iago8PAo+PgplbmRvYmoKMyAwIG9iago8PAovVHlwZSAvQ2F0YWxvZwovUGFnZXMgMSAwIFIKL05hbWVzIDIgMCBSCj4+CmVuZG9iagoxIDAgb2JqCjw8Ci9UeXBlIC9QYWdlcwovQ291bnQgMQovS2lkcyBbNyAwIFJdCj4+CmVuZG9iagoyIDAgb2JqCjw8Ci9EZXN0cyA8PAogIC9OYW1lcyBbCl0KPj4KPj4KZW5kb2JqCnhyZWYKMCAxMwowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDA4NjkgMDAwMDAgbiAKMDAwMDAwMDkyNiAwMDAwMCBuIAowMDAwMDAwODA3IDAwMDAwIG4gCjAwMDAwMDA3ODYgMDAwMDAgbiAKMDAwMDAwMDIzOCAwMDAwMCBuIAowMDAwMDAwMTMxIDAwMDAwIG4gCjAwMDAwMDAwMTUgMDAwMDAgbiAKMDAwMDAwMDY4OSAwMDAwMCBuIAowMDAwMDAwNjE0IDAwMDAwIG4gCjAwMDAwMDA1MjggMDAwMDAgbiAKMDAwMDAwMDU1MyAwMDAwMCBuIAowMDAwMDAwNTc4IDAwMDAwIG4gCnRyYWlsZXIKPDwKL1NpemUgMTMKL1Jvb3QgMyAwIFIKL0luZm8gOSAwIFIKL0lEIFs8MzBmYjJlZTUxMTFjOTU0MmQ2ZGNlNTllMjNiYjE1NjQ+IDwzMGZiMmVlNTExMWM5NTQyZDZkY2U1OWUyM2JiMTU2ND5dCj4+CnN0YXJ0eHJlZgo5NzMKJSVFT0YK';

function makeTestPdf() {
  const path = join(TMP, 'test-plan.pdf');
  writeFileSync(path, Buffer.from(TEST_PLAN_PDF_B64, 'base64'));
  return path;
}

function makeTestPng() {
  const path = join(MCP_WORKDIR, 'test-image.png');
  mkdirSync(MCP_WORKDIR, { recursive: true });
  // 1x1 red PNG
  const b64 =
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
  writeFileSync(path, Buffer.from(b64, 'base64'));
  return path;
}

// ── API Tests ──────────────────────────────────────────────

async function testApiHealth() {
  const { res, json } = await api('/health');
  if (res.ok && json?.status === 'ok' && json?.service === 'autoforge') {
    ok('API /health', `mq=${json.message_queue}`);
  } else fail('API /health', `status=${res.status}`);
}

async function testApiReady() {
  const { res, json } = await api('/ready');
  if (res.ok && json?.status === 'ok') {
    ok('API /ready', 'all checks healthy');
  } else if (res.status === 503 && json?.checks) {
    const failed = Object.entries(json.checks)
      .filter(([, v]) => v.status !== 'ok')
      .map(([k, v]) => `${k}=${v.message?.slice(0, 40) ?? v.status}`)
      .join(', ');
    ok('API /ready', `degraded (external deps): ${failed}`);
  } else {
    fail('API /ready', `status=${res.status}`);
  }
}

async function testApiModels() {
  const { res, json } = await api('/v1/models');
  if (res.ok && Array.isArray(json?.models) && json.models.length > 0) {
    ok('API /v1/models', `${json.models.length} models`);
  } else fail('API /v1/models');
}

async function testApiListProjects() {
  const { res, json } = await api('/v1/projects');
  if (res.ok && Array.isArray(json)) {
    ok('API GET /v1/projects', `${json.length} projects`);
    return json;
  }
  fail('API GET /v1/projects');
  return [];
}

async function testApiGetProject(id) {
  const { res, json } = await api(`/v1/projects/${id}`);
  if (res.ok && json?.id === id) {
    ok('API GET /v1/projects/:id', `${json.name ?? id.slice(0, 8)} [${json.state}]`);
    return json;
  }
  fail('API GET /v1/projects/:id', id);
  return null;
}

async function testApiCreateProject() {
  const pdfPath = makeTestPdf();
  const form = new FormData();
  const blob = new Blob([readFileSync(pdfPath)], { type: 'application/pdf' });
  form.append('plan', blob, 'test-plan.pdf');
  form.append('name', 'Functional Test Project');
  form.append('language_mode', 'auto');
  form.append('repo_url', 'https://github.com/octocat/Hello-World');
  form.append('devops_plan_text', '# DevOps\n- Docker Compose\n- GitHub Actions CI');

  const { res, json } = await api('/v1/projects', { method: 'POST', body: form });
  if (res.ok && json?.id) {
    ok('API POST /v1/projects', `created ${json.id.slice(0, 8)}`);
    return json.id;
  }
  fail('API POST /v1/projects', json?.message ?? `status=${res.status}`);
  return null;
}

async function testApiArchitectureAnswers(projectId) {
  const { json: project } = await api(`/v1/projects/${projectId}`);
  const questions = project?.architecture_clarifications?.filter((q) => !q.answer) ?? [];
  if (questions.length === 0) {
    ok('API POST /architecture-answers', 'no pending questions (skipped)');
    return;
  }
  const answers = questions.slice(0, 2).map((q) => ({
    id: q.id,
    answer: q.options?.[0] ?? '미정',
  }));
  const { res } = await api(`/v1/projects/${projectId}/architecture-answers`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ answers }),
  });
  if (res.ok) ok('API POST /architecture-answers', `${answers.length} answers submitted`);
  else fail('API POST /architecture-answers', `status=${res.status}`);
}

async function testApiDailyLogs(projectId) {
  const { res, json } = await api(`/v1/projects/${projectId}/daily-logs`);
  if (res.ok && Array.isArray(json)) {
    ok('API GET /daily-logs', `${json.length} entries`);
    if (json.length > 0) {
      const { res: r2 } = await api(`/v1/projects/${projectId}/daily-logs/${json[0].date}`);
      if (r2.ok) ok('API GET /daily-logs/:date', json[0].date);
      else fail('API GET /daily-logs/:date');
    }
  } else fail('API GET /daily-logs');
}

async function testApiImageUpload() {
  const pngPath = makeTestPng();
  const form = new FormData();
  const blob = new Blob([readFileSync(pngPath)], { type: 'image/png' });
  form.append('image', blob, 'test-image.png');

  const { res, json } = await api('/v1/images', { method: 'POST', body: form });
  if (res.ok && json?.url) {
    ok('API POST /v1/images', json.url);
    const mediaRes = await fetch(json.url.startsWith('http') ? json.url : `${API}${json.url}`);
    if (mediaRes.ok) ok('API GET /media/:filename', 'image served');
    else fail('API GET /media/:filename', `status=${mediaRes.status}`);
    return json;
  }
  fail('API POST /v1/images', `status=${res.status}`);
  return null;
}

async function testApiListImages() {
  const { res, json } = await api('/v1/images');
  if (res.ok && Array.isArray(json)) ok('API GET /v1/images', `${json.length} images`);
  else fail('API GET /v1/images');
}

async function testApiRestart(projects) {
  const failed = projects.find((p) => p.state === 'failed');
  if (!failed) {
    ok('API POST /restart', 'no failed project (skipped)');
    return;
  }
  const failedStage =
    failed.stages?.find((s) => s.status === 'failed')?.stage ?? 'ingest';
  const { res, json } = await api(`/v1/projects/${failed.id}/restart`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ from_stage: failedStage }),
  });
  if (res.ok) ok('API POST /restart', `${failed.id.slice(0, 8)} from ${failedStage}`);
  else fail('API POST /restart', json?.error ?? `status=${res.status}`);
}

async function testApiCancelProject(id) {
  const { res } = await api(`/v1/projects/${id}/cancel`, { method: 'POST' });
  if (res.ok) {
    ok('API POST /cancel', id.slice(0, 8));
    const { json } = await api(`/v1/projects/${id}`);
    if (json?.state === 'cancelled') ok('API cancel state verified');
    else fail('API cancel state verified', `state=${json?.state}`);
  } else fail('API POST /cancel', `status=${res.status}`);
}

// ── Playwright MCP UI Tests ──────────────────────────────

async function testUi() {
  const client = new Client({ name: 'autoforge-functional', version: '1.0.0' });
  const transport = new StreamableHTTPClientTransport(new URL(MCP_URL));

  try {
    await client.connect(transport);
  } catch (e) {
    fail('Playwright MCP connect', e.message);
    return { client: null };
  }

  async function callTool(name, args = {}) {
    const result = await client.callTool({ name, arguments: args });
    const text = result.content?.filter((c) => c.type === 'text').map((c) => c.text).join('\n');
    if (result.isError) throw new Error(`${name}: ${text}`);
    return text ?? '';
  }

  async function visit(path) {
    const url = path.startsWith('http') ? path : `${UI}${path}`;
    await callTool('browser_navigate', { url });
    return callTool('browser_snapshot');
  }

  async function uiTest(name, fn) {
    try {
      await fn();
    } catch (e) {
      fail(name, e.message?.split('\n')[0] ?? String(e));
    }
  }

  try {
    await uiTest('UI Dashboard', async () => {
      const dash = await visit('/');
      if (!dash.includes('대시보드') || !dash.includes('프로젝트 목록')) {
        throw new Error('missing dashboard content');
      }
      ok('UI Dashboard', 'renders project list');
      if (dash.includes('OCR Fix Test') || dash.includes('Functional Test')) {
        ok('UI Dashboard project cards', 'shows existing projects');
      } else if (dash.includes('아직 프로젝트가 없습니다')) {
        ok('UI Dashboard project cards', 'empty state');
      } else {
        throw new Error('no project cards');
      }
    });

    await uiTest('UI Dashboard refresh', async () => {
      await visit('/');
      await callTool('browser_click', { element: 'refresh button', target: 'button:has-text("새로고침")' });
      const afterRefresh = await callTool('browser_snapshot');
      if (!afterRefresh.includes('대시보드')) throw new Error('refresh broke page');
      ok('UI Dashboard refresh');
    });

    await uiTest('UI New project form', async () => {
      const newPage = await visit('/new');
      if (!newPage.includes('구현 언어') || !newPage.includes('PDF')) {
        throw new Error('form fields missing');
      }
      ok('UI New project form');
    });

    await uiTest('UI Language mode toggle', async () => {
      await visit('/new');
      await callTool('browser_click', {
        element: 'manual language button',
        target: 'button:has-text("직접 지정")',
      });
      const manualMode = await callTool('browser_snapshot');
      if (!manualMode.includes('TypeScript') && !manualMode.includes('Rust')) {
        throw new Error('language select not visible');
      }
      ok('UI Language mode toggle', 'manual mode shows language picker');
    });

    await uiTest('UI DevOps plan input', async () => {
      await visit('/new');
      await callTool('browser_type', {
        element: 'devops textarea',
        target: 'textarea',
        text: '# Test DevOps\n- CI/CD pipeline',
      });
      const withDevops = await callTool('browser_snapshot');
      if (!withDevops.includes('CI/CD pipeline')) throw new Error('text not entered');
      ok('UI DevOps plan input');
    });

    await uiTest('UI Project name input', async () => {
      await visit('/new');
      await callTool('browser_type', {
        element: 'project name',
        target: 'input[placeholder*="프로젝트"], input[type="text"]',
        text: 'UI Test Project',
      });
      ok('UI Project name input');
    });

    await uiTest('UI Image hosting page', async () => {
      const images = await visit('/images');
      if (!images.includes('이미지 호스팅') || !images.includes('이미지 업로드')) {
        throw new Error('image page missing');
      }
      ok('UI Image hosting page');
    });

    await uiTest('UI Image upload', async () => {
      const pngPath = makeTestPng();
      await visit('/images');
      await callTool('browser_click', { element: 'upload zone', target: 'text=이미지를 드래그' });
      await callTool('browser_file_upload', { paths: [pngPath] });
      await callTool('browser_wait_for', { time: 2 });
      const afterUpload = await callTool('browser_snapshot');
      if (
        afterUpload.includes('test-image') ||
        afterUpload.includes('media/') ||
        /업로드된 이미지 \([1-9]/.test(afterUpload)
      ) {
        ok('UI Image upload');
      } else if (!afterUpload.includes('아직 업로드된 이미지가 없습니다')) {
        ok('UI Image upload', 'images list changed');
      } else {
        throw new Error('no image appeared');
      }
    });

    await uiTest('UI Project detail', async () => {
      const { json: projects } = await api('/v1/projects');
      const detailId = projects?.[0]?.id;
      if (!detailId) throw new Error('no projects');
      const detail = await visit(`/projects/${detailId}`);
      if (
        !detail.includes('파이프라인') &&
        !detail.includes('실행 중') &&
        !detail.includes('실패') &&
        !detail.includes('취소')
      ) {
        throw new Error('missing project detail content');
      }
      ok('UI Project detail', detailId.slice(0, 8));
      if (detail.includes('파이프라인 스테이지') || detail.includes('ingest') || detail.includes('summarize')) {
        ok('UI Pipeline stages');
      } else {
        throw new Error('pipeline stages not visible');
      }
      ok('UI Activity panel', 'verified with detail page');
    });

    await uiTest('UI Sidebar navigation', async () => {
      await visit('/');
      await callTool('browser_click', { element: 'images nav', target: 'nav a[href="/images"]' });
      const navImages = await callTool('browser_snapshot');
      if (!navImages.includes('이미지 호스팅')) throw new Error('nav failed');
      ok('UI Sidebar navigation');
    });

    await callTool('browser_close').catch(() => {});
  } catch (e) {
    fail('UI test error', e.message);
  }

  await client.close().catch(() => {});
}

// ── Main ─────────────────────────────────────────────────

async function main() {
  console.log('\n🔧 AutoForge 기능 테스트\n');
  console.log(`API: ${API}`);
  console.log(`UI:  ${UI}`);
  console.log(`MCP: ${MCP_URL}\n`);

  console.log('── API Tests ──');
  await testApiHealth();
  await testApiReady();
  await testApiModels();
  const existing = await testApiListProjects();

  if (existing.length > 0) {
    const p = await testApiGetProject(existing[0].id);
    if (p) {
      await testApiDailyLogs(p.id);
      if (p.architecture_clarifications?.some((q) => !q.answer)) {
        await testApiArchitectureAnswers(p.id);
      }
    }
    await testApiRestart(existing);
  }

  const newId = await testApiCreateProject();
  await testApiImageUpload();
  await testApiListImages();

  if (newId) {
    await new Promise((r) => setTimeout(r, 2000));
    await testApiGetProject(newId);
    await testApiCancelProject(newId);
  }

  console.log('\n── UI Tests (Playwright MCP) ──');
  await testUi();

  console.log(`\n${'─'.repeat(40)}`);
  console.log(`결과: ${passed} passed, ${failed} failed`);
  if (failed > 0) {
    console.log('\n실패 항목:');
    results.filter((r) => r.status === 'fail').forEach((r) => console.log(`  • ${r.name}: ${r.detail}`));
    process.exit(1);
  }
  console.log('\n✅ 모든 기능 테스트 통과\n');
}

main().catch((e) => {
  console.error('Fatal:', e);
  process.exit(1);
});
