import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { getProject, listProjects } from '../src/api/client';
import { subscribeProjectWebSocket, subscribeProjectsWebSocket } from '../src/api/ws';
import { useProject } from '../src/hooks/useProject';
import { useProjects } from '../src/hooks/useProjects';
import type { ProjectDetail } from '../src/types';

vi.mock('../src/api/client', () => ({ getProject: vi.fn(), listProjects: vi.fn() }));
vi.mock('../src/api/ws', () => ({ subscribeProjectWebSocket: vi.fn(), subscribeProjectsWebSocket: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const project = (id: string) => ({ id, name: id }) as ProjectDetail;
let root: Root;
let container: HTMLDivElement;
let detail: ReturnType<typeof useProject>;
let list: ReturnType<typeof useProjects>;
function Detail({ id }: { id?: string }) { detail = useProject(id); return null; }
function List() { list = useProjects(); return null; }

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  container = document.createElement('div');
  root = createRoot(container);
  vi.mocked(subscribeProjectWebSocket).mockReturnValue(vi.fn());
  vi.mocked(subscribeProjectsWebSocket).mockReturnValue(vi.fn());
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
});

it('ignores an old project response after navigation and clears old content', async () => {
  const old = deferred<ProjectDetail>();
  const current = deferred<ProjectDetail>();
  vi.mocked(getProject).mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  await act(async () => root.render(<Detail id="old" />));
  await act(async () => root.render(<Detail id="current" />));
  await act(async () => current.resolve(project('current')));
  await act(async () => old.resolve(project('old')));
  expect(detail.project?.id).toBe('current');
  expect(detail.loading).toBe(false);
  vi.mocked(getProject).mockReturnValue(new Promise(() => {}));
  await act(async () => root.render(<Detail id="next" />));
  expect(detail.project).toBeNull();
  expect(detail.loading).toBe(true);
});

it('preserves live detail updates when an earlier HTTP request fails', async () => {
  const request = deferred<ProjectDetail>();
  vi.mocked(getProject).mockReturnValue(request.promise);
  await act(async () => root.render(<Detail id="live" />));
  await act(async () => vi.mocked(subscribeProjectWebSocket).mock.calls[0][1](project('live')));
  await act(async () => request.reject(new Error('stale failure')));
  expect(detail.project?.id).toBe('live');
  expect(detail.error).toBeNull();
  expect(detail.loading).toBe(false);
});

it('preserves live list updates over an older HTTP snapshot', async () => {
  const request = deferred<ProjectDetail[]>();
  vi.mocked(listProjects).mockReturnValue(request.promise);
  await act(async () => root.render(<List />));
  await act(async () => vi.mocked(subscribeProjectsWebSocket).mock.calls[0][0]([project('live')]));
  await act(async () => request.resolve([project('stale')]));
  expect(list.projects[0].id).toBe('live');
});

it('only applies the latest of overlapping manual refresh requests', async () => {
  vi.mocked(listProjects).mockResolvedValue([]);
  await act(async () => root.render(<List />));
  const earlier = deferred<ProjectDetail[]>();
  const latest = deferred<ProjectDetail[]>();
  vi.mocked(listProjects).mockReturnValueOnce(earlier.promise).mockReturnValueOnce(latest.promise);
  let first!: Promise<void>;
  let second!: Promise<void>;
  await act(async () => { first = list.refresh(); second = list.refresh(); });
  await act(async () => { latest.resolve([project('latest')]); await second; });
  await act(async () => { earlier.resolve([project('earlier')]); await first; });
  expect(list.projects[0].id).toBe('latest');
});

it('clears data and finishes loading when no project is selected', async () => {
  vi.mocked(getProject).mockResolvedValue(project('selected'));
  await act(async () => root.render(<Detail id="selected" />));
  await act(async () => root.render(<Detail />));
  expect(detail.project).toBeNull();
  expect(detail.loading).toBe(false);
  expect(detail.error).toBeNull();
});

it('loads HTTP data when WebSocket is unavailable and exposes current errors', async () => {
  vi.stubGlobal('WebSocket', undefined);
  vi.mocked(listProjects).mockRejectedValue(new Error('offline'));
  await act(async () => root.render(<List />));
  expect(list.error).toBe('offline');
  expect(list.loading).toBe(false);
  expect(subscribeProjectsWebSocket).not.toHaveBeenCalled();
  vi.mocked(listProjects).mockResolvedValue([project('recovered')]);
  await act(async () => list.refresh());
  expect(list.error).toBeNull();
  expect(list.projects[0].id).toBe('recovered');
});

it('unsubscribes and ignores saved refresh callbacks after unmount', async () => {
  vi.mocked(listProjects).mockResolvedValue([]);
  await act(async () => root.render(<List />));
  const refresh = list.refresh;
  const unsubscribe = vi.mocked(subscribeProjectsWebSocket).mock.results[0].value;
  await act(async () => root.render(null));
  await refresh();
  expect(unsubscribe).toHaveBeenCalledOnce();
  expect(listProjects).toHaveBeenCalledOnce();
});
