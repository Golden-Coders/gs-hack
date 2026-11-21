// Kolejka zatwierdzeń — tasks.md 14.3.
// Zatwierdzać i odrzucać można tylko tutaj (z UI); autorem jest wybrana tożsamość („działam jako”).
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import "./Approvals.css";

type Pending = Schemas["PendingApproval"];
type Decision = "approve" | "reject";

// Komenda trafia do runbooków asynchronnie (202) – przez chwilę lista z serwera może jeszcze zawierać krok,
// który właśnie zatwierdziliśmy. Ukrywamy go lokalnie, ale tylko na krótko (gdyby komenda przepadła).
const DECIDED_HIDE_MS = 15_000;

const keyOf = (a: Pending) => `${a.run_id}:${a.step_id}`;

const dateFmt = new Intl.DateTimeFormat("pl-PL", { dateStyle: "short", timeStyle: "medium" });

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
  return `${ago} (${dateFmt.format(new Date(iso))})`;
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
  const [items, setItems] = useState<Pending[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [confirmReject, setConfirmReject] = useState<string | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const decided = useRef(new Map<string, number>());

  const load = useCallback(async () => {
    try {
      const { data, error } = await api.GET("/approvals");
      if (error || !data) throw new Error("approvals");
      setLoadError(null);
      const now = Date.now();
      setItems(data.filter((a) => now - (decided.current.get(keyOf(a)) ?? -Infinity) > DECIDED_HIDE_MS));
    } catch {
      setLoadError("Nie udało się pobrać kolejki zatwierdzeń.");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Na żywo: zmiana statusu kroku (np. zatwierdzenie z innej karty) odświeża kolejkę.
  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "step_status") void load();
  });

  usePollWhenQuiet(() => void load());

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
        // Optymistycznie zdejmujemy krok z kolejki; odświeżenie potwierdzi stan z serwera.
        decided.current.set(key, Date.now());
        setItems((cur) => cur?.filter((x) => keyOf(x) !== key) ?? cur);
        setConfirmReject(null);
      } else {
        const msg =
          response.status === 409
            ? "Krok nie czeka już na zatwierdzenie (ktoś był szybszy lub runbook się zmienił)."
            : response.status === 404
              ? "Nie znaleziono kroku — mógł zostać zamknięty."
              : `Błąd ${response.status} przy wysyłaniu decyzji.`;
        setRowErrors((e) => ({ ...e, [key]: msg }));
        if (response.status === 409 || response.status === 404) void load();
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
        <button type="button" onClick={() => void load()}>
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
        <p className="error-box" role="alert">
          Wybierz tożsamość („działam jako” w panelu bocznym), aby zatwierdzać i odrzucać kroki.
        </p>
      )}
      {loadError && (
        <p className="error-box" role="alert">
          {loadError}
        </p>
      )}

      {items === null && !loadError && <p className="muted">Ładowanie…</p>}
      {items?.length === 0 && <p className="muted approvals-empty">Brak kroków do zatwierdzenia</p>}

      {items && items.length > 0 && (
        <ul className="approvals-list" aria-label="Kroki oczekujące na zatwierdzenie">
          {items.map((a) => {
            const key = keyOf(a);
            const isBusy = busy.has(key);
            const confirming = confirmReject === key;
            const high = a.risk === "high";
            return (
              <li key={key} className={`approval${high ? " approval-high" : ""}`}>
                <div className="approval-main">
                  <div className="approval-title">
                    <span className={`risk risk-${a.risk}`}>{high ? "Ryzyko: WYSOKIE" : "Ryzyko: niskie"}</span>
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
                  {rowErrors[key] && (
                    <p className="error-box" role="alert">
                      {rowErrors[key]}
                    </p>
                  )}
                </div>
                <div className="approval-actions">
                  {confirming ? (
                    <>
                      <span className="approval-confirm-text">Odrzucić ten krok?</span>
                      <button
                        type="button"
                        className="danger"
                        disabled={isBusy || !personId}
                        onClick={() => void decide(a, "reject")}
                      >
                        {isBusy ? "Wysyłanie…" : "Tak, odrzuć"}
                      </button>
                      <button type="button" disabled={isBusy} onClick={() => setConfirmReject(null)}>
                        Anuluj
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="primary"
                        disabled={isBusy || !personId}
                        onClick={() => void decide(a, "approve")}
                      >
                        {isBusy ? "Wysyłanie…" : "Zatwierdź"}
                      </button>
                      <button
                        type="button"
                        className="danger"
                        disabled={isBusy || !personId}
                        onClick={() => setConfirmReject(key)}
                      >
                        Odrzuć
                      </button>
                    </>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
