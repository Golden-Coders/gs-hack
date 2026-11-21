import { useCallback, useEffect, useRef, useState } from "react";
import { describeError } from "./request";

export type FetchResult<T> = { data?: T; error?: unknown; response: Response };

export type Resource<T> = {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
  /** Ciche odświeżenie (dociąganie w tle): bez wskaźnika ładowania. */
  refresh: () => void;
};

/**
 * Ładuje jedną sekcję ekranu. `key` zmienia się wraz z alertem (reset danych);
 * `reload` dociąga dane bez czyszczenia (widok nie mruga przy odświeżeniu z SSE).
 */
export function useResource<T>(load: () => Promise<FetchResult<T>>, key = ""): Resource<T> {
  const [state, setState] = useState<{ data: T | null; error: string | null; loading: boolean }>({
    data: null,
    error: null,
    loading: true,
  });
  const loadRef = useRef(load);
  loadRef.current = load;
  const seq = useRef(0);

  const fetchData = useCallback((silent: boolean) => {
    const n = ++seq.current;
    if (!silent) setState((s) => ({ ...s, loading: true }));
    loadRef.current().then(
      ({ data, error, response }) => {
        if (n !== seq.current) return; // starsza odpowiedź wyprzedzona nowszym żądaniem
        if (error !== undefined || data === undefined) {
          setState((s) => ({ ...s, error: describeError(error, response), loading: false }));
        } else {
          setState({ data, error: null, loading: false });
        }
      },
      () => {
        if (n === seq.current) setState((s) => ({ ...s, error: "Cannot connect to API", loading: false }));
      },
    );
  }, []);
  const reload = useCallback(() => fetchData(false), [fetchData]);
  const refresh = useCallback(() => fetchData(true), [fetchData]);

  useEffect(() => {
    setState({ data: null, error: null, loading: true });
    reload();
    const s = seq; // ten sam obiekt ref; kopia tylko dla react-hooks/exhaustive-deps
    return () => {
      s.current++;
    };
  }, [key, reload]);

  return { ...state, reload, refresh };
}
