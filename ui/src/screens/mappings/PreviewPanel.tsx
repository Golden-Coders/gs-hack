// Podgląd wyniku mappingu na przykładowym zdarzeniu: POST /mappings/preview (z odświeżaniem po zmianach),
// a gdy backend nie ma jeszcze podglądu (501) – przybliżony wynik liczony lokalnie, wyraźnie oznaczony.
import { useEffect, useRef, useState } from "react";
import { api, type Schemas } from "../../api/client";
import { SeverityBadge } from "../../components/Badges";
import { formatValue } from "../../components/Presentation";
import type { Mapping } from "./draft";
import { localPreview, type PreviewResult } from "./localPreview";

type State =
  | { kind: "idle" }
  | { kind: "loading"; last?: PreviewResult }
  | { kind: "ok"; result: PreviewResult }
  | { kind: "error"; message: string };

function fromServer(e: Schemas["CanonicalEvent"]): PreviewResult {
  return {
    severity: e.severity,
    title: e.title,
    fingerprint: e.fingerprint,
    resolved: e.resolved,
    fields: e.fields,
    origin: "server",
    mappingVersion: e.mapping_version,
  };
}

const DEBOUNCE_MS = 400;

export default function PreviewPanel({
  mapping,
  payload,
  blocked,
}: {
  mapping: Mapping;
  payload: unknown;
  /** Powód, dla którego podgląd nie może się policzyć (np. brak payloadu, błędy w formularzu). */
  blocked?: string;
}) {
  const [state, setState] = useState<State>({ kind: "idle" });
  const [tick, setTick] = useState(0);
  const seq = useRef(0);
  const key = JSON.stringify([mapping, payload, tick]);

  useEffect(() => {
    if (blocked) return;
    const mine = ++seq.current;
    setState((s) => ({ kind: "loading", last: s.kind === "ok" ? s.result : s.kind === "loading" ? s.last : undefined }));
    const t = setTimeout(async () => {
      let next: State;
      try {
        const { data, error, response } = await api.POST("/mappings/preview", { body: { mapping, payload } });
        if (data) next = { kind: "ok", result: fromServer(data) };
        else if (response.status === 400) {
          next = { kind: "error", message: [error?.error, error?.detail].filter(Boolean).join(": ") || "Mapping odrzucony przez serwer (400)." };
        } else {
          // 501 (podgląd niezaimplementowany), 404/405 albo inny błąd serwera – zostaje wynik lokalny
          next = { kind: "ok", result: localPreview(mapping, payload) };
        }
      } catch {
        next = { kind: "ok", result: localPreview(mapping, payload) };
      }
      if (mine === seq.current) setState(next);
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` obejmuje mapping, payload i ręczne odświeżenie
  }, [key, blocked]);

  const shown: PreviewResult | undefined =
    state.kind === "ok" ? state.result : state.kind === "loading" ? state.last : undefined;

  return (
    <div className="mp-preview" aria-live="polite">
      {blocked ? (
        <p className="muted">{blocked}</p>
      ) : (
        <>
          <div className="mp-preview-bar">
            <span className="muted">
              {state.kind === "loading" ? "Liczenie podglądu…" : shown ? "Wynik dla przykładowego zdarzenia" : ""}
            </span>
            <button type="button" onClick={() => setTick((n) => n + 1)}>
              Odśwież podgląd
            </button>
          </div>
          {state.kind === "error" && <p className="error-box">{state.message}</p>}
          {shown && state.kind !== "error" && (
            <>
              {shown.origin === "local" && (
                <p className="mp-note">
                  Podgląd lokalny (przybliżony): serwer nie udostępnia jeszcze podglądu mappingu. Zapisany mapping
                  zadziała według reguł serwera.
                </p>
              )}
              <dl className="mp-result">
                <dt>Severity</dt>
                <dd><SeverityBadge severity={shown.severity} /></dd>
                <dt>Tytuł</dt>
                <dd>{shown.title || <span className="muted">(pusty)</span>}</dd>
                <dt>Fingerprint</dt>
                <dd className="mono">{shown.fingerprint}</dd>
                <dt>Rozwiązuje alert</dt>
                <dd>{shown.resolved ? "tak (resolved)" : "nie"}</dd>
                <dt>Pola</dt>
                <dd>
                  {Object.keys(shown.fields).length === 0 ? (
                    <span className="muted">brak dodatkowych pól</span>
                  ) : (
                    <ul className="mp-result-fields">
                      {Object.entries(shown.fields).map(([k, v]) => (
                        <li key={k}>
                          <span className="mono">{k}</span>
                          {v === null ? <span className="muted">pusta wartość (pole nie istnieje w payloadzie)</span> : <span>{formatValue(v)}</span>}
                        </li>
                      ))}
                    </ul>
                  )}
                </dd>
              </dl>
            </>
          )}
        </>
      )}
    </div>
  );
}
