#!/usr/bin/env node
/**
 * Playwright MCP HTTP 서버에 연결해 AutoForge 프론트/백엔드 스모크 테스트를 수행한다.
 * 사용법: node scripts/e2e-playwright-mcp.mjs [baseUrl]
 */
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const BASE_URL = process.argv[2] ?? 'http://localhost:5173';
const MCP_URL = process.env.PLAYWRIGHT_MCP_URL ?? 'http://localhost:8931/mcp';

const client = new Client({ name: 'autoforge-e2e', version: '1.0.0' });
const transport = new StreamableHTTPClientTransport(new URL(MCP_URL));

async function callTool(name, args = {}) {
  const result = await client.callTool({ name, arguments: args });
  const text = result.content
    ?.filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n');
  if (result.isError) {
    throw new Error(`${name} failed: ${text ?? JSON.stringify(result)}`);
  }
  return text ?? '';
}

async function visit(path, label) {
  const url = path.startsWith('http') ? path : `${BASE_URL}${path}`;
  console.log(`→ ${label}: ${url}`);
  await callTool('browser_navigate', { url });
  return callTool('browser_snapshot');
}

function assertIncludes(haystack, needle, label) {
  if (!haystack.includes(needle)) {
    throw new Error(`${label}: expected to include "${needle}"`);
  }
}

async function main() {
  await client.connect(transport);

  const dash = await visit('/', 'dashboard');
  assertIncludes(dash, '대시보드', 'dashboard page');
  assertIncludes(dash, '프로젝트 목록', 'dashboard project list');
  assertIncludes(dash, '전체 프로젝트', 'dashboard stats');

  const newPage = await visit('/new', 'new project page');
  assertIncludes(newPage, '새 프로젝트', 'new project heading');
  assertIncludes(newPage, '구현 언어', 'language section');
  assertIncludes(newPage, '자동 선택', 'auto language mode');
  assertIncludes(newPage, '한국 산업 맥락', 'korean industry context hint');

  const images = await visit('/images', 'image hosting');
  assertIncludes(images, '이미지 호스팅', 'image hosting page');
  assertIncludes(images, '이미지 업로드', 'image upload section');

  const health = await visit('/health', 'API health (proxied)');
  assertIncludes(health, 'status', 'health endpoint body');
  assertIncludes(health, 'autoforge', 'health service name');

  const projects = await visit('/v1/projects', 'API projects (proxied)');
  assertIncludes(projects, '[', 'projects json array');
  assertIncludes(projects, 'OCR Fix Test', 'existing project visible in API');

  console.log('→ sidebar navigation click test');
  await callTool('browser_navigate', { url: BASE_URL });
  const clickResult = await callTool('browser_click', {
    element: 'sidebar new project link',
    target: 'nav a[href="/new"]',
  });
  const afterClick = await callTool('browser_snapshot');
  assertIncludes(afterClick, '구현 언어', 'navigated to new project via click');

  await callTool('browser_close').catch(() => {});
  await client.close();
  console.log('\n✅ Playwright MCP E2E smoke test passed');
}

main().catch((err) => {
  console.error('\n❌ Playwright MCP E2E failed:', err.message);
  process.exit(1);
});
