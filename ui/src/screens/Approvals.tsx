// Kolejka zatwierdzeń — tasks.md 14.3.
// Zatwierdzać i odrzucać można tylko tutaj (z UI); autorem jest wybrana tożsamość („działam jako”).
import { useState } from "react";
import { Link } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { RiskBadge } from "../components/Badges";
import { formatTime } from "../ui/format";
import { ConfirmInline, Empty, ErrorBox, Loading } from "../ui/primitives";
import "./Approvals.css";

type Pending = Schemas["PendingApproval"];
type Decision = "approve" | "reject";

// Komenda trafia do runbooków asynchronnie (202) – przez chwilę lista z serwera może jeszcze zawierać krok,
// który właśnie zatwierdziliśmy. Ukrywamy go lokalnie, ale tylko na krótko (gdyby komenda przepadła).
const DECIDED_HIDE_MS = 15_000;

const keyOf = (a: Pending) => `${a.run_id}:${a.step_id}`;

function formatSince(iso: string): string {
  const ms = Date.now() - Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  const min = Math.floor(ms / 60_000);
  const ago =
    min < 1
      ? "przed chwilą"
      : min < 60
        ? `${min} min temu`
        : min < 48 * 60
          ? `${Math.floor(min / 60)} h ${min % 60} min temu`
          : `${Math.floor(min / (24 * 60))} dni temu`;
  return `${ago} (${formatTime(iso)})`;
}

function Params({ params }: { params: Pending["params"] }) {
  const entries = Object.entries(params ?? {});
  if (entries.length === 0) return <span className="muted">brak parametrów</span>;
  return (
    <dl className="approval-params">
      {entries.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{typeof v === "string" ? v : JSON.stringify(v)}</dd>
        </div>
      ))}
    </dl>
  );
}

export default function Approvals() {
  const personId = usePersonId();
  const list = useResource(() => api.GET("/approvals"));
  const [decidedAt, setDecidedAt] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});

  // Optymistycznie ukrywamy krok, który właśnie zatwierdziliśmy; odświeżenie potwierdzi stan z serwera.
  const now = Date.now();
  const items = list.data?.filter((a) => now - (decidedAt[keyOf(a)] ?? -Infinity) > DECIDED_HIDE_MS) ?? null;

  // Na żywo: zmiana statusu kroku (np. zatwierdzenie z innej karty) odświeża kolejkę.
  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "step_status") list.refresh();
  });

  usePollWhenQuiet(list.refresh);

  async function decide(a: Pending, command: Decision) {
    const key = keyOf(a);
    setBusy((s) => new Set(s).add(key));
    setRowErrors((cur) => {
      const rest = { ...cur };
      delete rest[key];
      return rest;
    });
    try {
      const { response } = await api.POST("/runs/{run_id}/steps/{step_id}/{command}", {
        params: { path: { run_id: a.run_id, step_id: a.step_id, command }, header: asPerson() },
      });
      if (response.ok) {
        setDecidedAt((cur) => ({ ...cur, [key]: Date.now() }));
      } else {
        const msg =
          response.status === 409
            ? "Krok nie czeka już na zatwierdzenie (ktoś był szybszy lub runbook się zmienił)."
            : response.status === 404
              ? "Nie znaleziono kroku — mógł zostać zamknięty."
              : `Błąd ${response.status} przy wysyłaniu decyzji.`;
        setRowErrors((e) => ({ ...e, [key]: msg }));
        if (response.status === 409 || response.status === 404) list.refresh();
      }
    } catch {
      setRowErrors((e) => ({ ...e, [key]: "Brak połączenia z API — decyzja nie została wysłana." }));
    } finally {
      setBusy((s) => {
        const n = new Set(s);
        n.delete(key);
        return n;
      });
    }
  }

  return (
    <section className="approvals">
      <header className="approvals-head">
        <h1>Kolejka zatwierdzeń</h1>
        <button type="button" onClick={list.reload}>
          Odśwież
        </button>
      </header>
      <p className="muted">
        Kroki runbooków czekające na decyzję człowieka. Zatwierdzenie jest możliwe tylko tutaj
        {personId ? (
          <>
            {" "}
            — decyzję zapiszemy jako <strong>{personId}</strong>.
          </>
        ) : (
          "."
        )}
      </p>

      {!personId && (
        <ErrorBox message="Wybierz tożsamość („działam jako” w panelu bocznym), aby zatwierdzać i odrzucać kroki." />
      )}
      {list.error && (
        <ErrorBox message={`Nie udało się pobrać kolejki zatwierdzeń: ${list.error}`} onRetry={list.reload} disabled={list.loading} />
      )}

      {items === null && !list.error && <Loading />}
      {items?.length === 0 && <Empty>Brak kroków do zatwierdzenia</Empty>}

      {items && items.length > 0 && (
        <ul className="approvals-list" aria-label="Kroki oczekujące na zatwierdzenie">
          {items.map((a) => {
            const key = keyOf(a);
            const isBusy = busy.has(key);
            return (
              <li key={key} className={`approval${a.risk === "high" ? " approval-high" : ""}`}>
                <div className="approval-main">
                  <div className="approval-title">
                    <RiskBadge risk={a.risk} />
                    <strong>{a.text}</strong>
                  </div>
                  <dl className="approval-meta">
                    <div>
                      <dt>Alert</dt>
                      <dd>
                        <Link to={`/alerts/${a.alert_id}`}>{a.alert_title}</Link>
                      </dd>
                    </div>
                    <div>
                      <dt>Runbook</dt>
                      <dd>{a.runbook_title}</dd>
                    </div>
                    <div>
                      <dt>Akcja</dt>
                      <dd>{a.action ? <code>{a.action}</code> : <span className="muted">brak (krok ręczny)</span>}</dd>
                    </div>
                    <div>
                      <dt>Czeka od</dt>
                      <dd>{formatSince(a.waiting_since)}</dd>
                    </div>
                    <div className="approval-params-row">
                      <dt>Parametry</dt>
                      <dd>
                        <Params params={a.params} />
                      </dd>
                    </div>
                  </dl>
                  {rowErrors[key] && <ErrorBox message={rowErrors[key]} />}
                </div>
                <div className="approval-actions">
                  <button
                    type="button"
                    className="primary"
                    disabled={isBusy || !personId}
                    onClick={() => void decide(a, "approve")}
                  >
                    {isBusy ? "Wysyłanie…" : "Zatwierdź"}
                  </button>
                  <ConfirmInline
                    label="Odrzuć"
                    question="Odrzucić ten krok?"
                    confirmLabel="Tak, odrzuć"
                    busy={isBusy}
                    disabled={!personId}
                    onConfirm={() => void decide(a, "reject")}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
