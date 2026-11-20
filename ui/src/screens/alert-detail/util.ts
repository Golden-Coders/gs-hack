// Pomocnicze funkcje ekranu szczegółów alertu: ładowanie sekcji, błędy, formatowanie.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Schemas } from "../../api/client";

type FetchResult<T> = { data?: T; error?: unknown; response: Response };

/** Opis błędu HTTP dla człowieka: komunikat z backendu (`Error.error`) albo sam status. */
export function describeError(error: unknown, response?: Response): string {
  const e = error as Partial<Schemas["Error"]> | undefined;
  const msg = e && typeof e.error === "string" ? e.error : null;
  const detail = e && typeof e.detail === "string" ? e.detail : null;
  const status = response ? `HTTP ${response.status}` : null;
  return [msg ?? status ?? "Nieznany błąd", detail].filter(Boolean).join(": ");
}

/** Wykonuje żądanie zmieniające stan; zwraca komunikat błędu albo `null`, gdy backend przyjął komendę. */
export async function send(request: () => Promise<{ error?: unknown; response: Response }>): Promise<string | null> {
  try {
    const { error, response } = await request();
    return error !== undefined || !response.ok ? describeError(error, response) : null;
  } catch {
    return "Brak połączenia z API";
  }
}

export type Resource<T> = { data: T | null; error: string | null; loading: boolean; reload: () => void };

/**
 * Ładuje jedną sekcję ekranu. `key` zmienia się wraz z alertem (reset danych);
 * `reload` dociąga dane bez czyszczenia (widok nie mruga przy odświeżeniu z SSE).
 */
export function useResource<T>(load: () => Promise<FetchResult<T>>, key: string): Resource<T> {
  const [state, setState] = useState<{ data: T | null; error: string | null; loading: boolean }>({
    data: null,
    error: null,
    loading: true,
  });
  const loadRef = useRef(load);
  loadRef.current = load;
  const seq = useRef(0);

  const reload = useCallback(() => {
    const n = ++seq.current;
    setState((s) => ({ ...s, loading: true }));
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
        if (n === seq.current) setState((s) => ({ ...s, error: "Brak połączenia z API", loading: false }));
      },
    );
  }, []);

  useEffect(() => {
    setState({ data: null, error: null, loading: true });
    reload();
    return () => {
      seq.current++;
    };
  }, [key, reload]);

  return { ...state, reload };
}

export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("pl-PL");
}

export function actorLabel(actor: Schemas["Actor"] | null | undefined): string {
  if (!actor) return "—";
  if (actor.type === "user") return actor.person_id ?? "użytkownik";
  return actor.type === "system" ? "system" : "źródło";
}
