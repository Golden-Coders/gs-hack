// Podgląd wyniku mappingu na przykładowym zdarzeniu: POST /mappings/preview (z odświeżaniem po zmianach),
// a gdy backend nie ma jeszcze podglądu (501) – przybliżony wynik liczony lokalnie, wyraźnie oznaczony.
import { useEffect, useRef, useState } from "react";
import Button from "@atlaskit/button/new";
import { Text } from "@atlaskit/primitives/compiled";
import SectionMessage from "@atlaskit/section-message";
import { api, type Schemas } from "../../api/client";
import { describeError } from "../../api/request";
import { formatValue } from "../../components/Presentation";
import { ErrorBox, StatusLozenge } from "../../ui/primitives";
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
          next = { kind: "error", message: describeError(error, response) };
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
        <Text as="p" color="color.text.subtle">{blocked}</Text>
      ) : (
        <>
          <div className="mp-preview-bar">
            <span className="muted">
              {state.kind === "loading" ? "Liczenie podglądu…" : shown ? "Wynik dla przykładowego zdarzenia" : ""}
            </span>
            <Button onClick={() => setTick((n) => n + 1)}>Odśwież podgląd</Button>
          </div>
          {state.kind === "error" && <ErrorBox message={state.message} />}
          {shown && state.kind !== "error" && (
            <>
              {shown.origin === "local" && (
                <SectionMessage appearance="warning">
                  Podgląd lokalny (przybliżony): serwer nie udostępnia jeszcze podglądu mappingu. Zapisany mapping
                  zadziała według reguł serwera.
                </SectionMessage>
              )}
              <dl className="mp-result">
                <dt>Severity</dt>
                <dd><StatusLozenge domain="severity" value={shown.severity} /></dd>
                <dt>Tytuł</dt>
                <dd>{shown.title || <Text color="color.text.subtle">(pusty)</Text>}</dd>
                <dt>Fingerprint</dt>
                <dd className="mono">{shown.fingerprint}</dd>
                <dt>Rozwiązuje alert</dt>
                <dd>{shown.resolved ? "tak (resolved)" : "nie"}</dd>
                <dt>Pola</dt>
                <dd>
                  {Object.keys(shown.fields).length === 0 ? (
                    <Text color="color.text.subtle">brak dodatkowych pól</Text>
                  ) : (
                    <ul className="mp-result-fields">
                      {Object.entries(shown.fields).map(([k, v]) => (
                        <li key={k}>
                          <span className="mono">{k}</span>
                          {v === null ? <Text color="color.text.subtle">pusta wartość (pole nie istnieje w payloadzie)</Text> : <span>{formatValue(v)}</span>}
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
