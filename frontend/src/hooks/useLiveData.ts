import { useCallback, useEffect, useRef, useState } from 'react';

/** Keep HTTP snapshots from overwriting newer requests or live updates. */
export function useLiveData<T>(
  load: (() => Promise<T>) | null,
  subscribe: (onUpdate: (data: T) => void) => () => void,
  initialData: T,
  failureMessage: string,
) {
  const [data, setData] = useState(initialData);
  const [loading, setLoading] = useState(load !== null);
  const [error, setError] = useState<string | null>(null);
  const version = useRef(0);
  const activeLoader = useRef<typeof load>(null);
  const invalidate = useCallback(() => ++version.current, []);

  const refresh = useCallback(async () => {
    if (!load || activeLoader.current !== load) return;
    const requestVersion = ++version.current;
    try {
      const result = await load();
      if (version.current !== requestVersion) return;
      setData(result);
      setError(null);
    } catch (e) {
      if (version.current !== requestVersion) return;
      setError(e instanceof Error ? e.message : failureMessage);
    } finally {
      if (version.current === requestVersion) setLoading(false);
    }
  }, [load, failureMessage]);

  useEffect(() => {
    let active = true;
    activeLoader.current = load;
    setData(initialData);
    setError(null);
    setLoading(load !== null);
    void refresh();
    const unsubscribe = load && typeof WebSocket !== 'undefined'
      ? subscribe((result) => {
          if (!active) return;
          invalidate();
          setData(result);
          setError(null);
          setLoading(false);
        })
      : undefined;

    return () => {
      active = false;
      activeLoader.current = null;
      invalidate();
      unsubscribe?.();
    };
  }, [load, subscribe, initialData, refresh, invalidate]);

  return { data, loading, error, refresh };
}
