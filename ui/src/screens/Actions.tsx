// Akcje — tasks.md 15.3: rejestr akcji (builtin tylko do odczytu), formularz akcji HTTP
// oraz lista akcji brakujących w rejestrze z przyciskiem „Dodaj”.
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { useStream } from "../api/stream";
import ActionForm from "./actions/ActionForm";
import { emptyForm, fromAction, type ActionDef, type FormState } from "./actions/model";
import "./Actions.css";

type Missing = Schemas["MissingAction"];

type FormTarget = { key: number; initial: FormState; isNew: boolean; version?: number };

// Rejestr przeładowuje się po NOTIFY w ciągu kilku sekund – świeżo dodaną nazwę ukrywamy na krótko lokalnie.
const ADDED_HIDE_MS = 15_000;

function RiskBadge({ risk }: { risk: ActionDef["risk"] }) {
  return <span className={`act-risk act-risk-${risk}`}>{risk === "high" ? "WYSOKIE" : "niskie"}</span>;
}

export default function Actions() {
  const [actions, setActions] = useState<ActionDef[] | null>(null);
  const [missing, setMissing] = useState<Missing[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormTarget | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Świeżo zapisane akcje: do czasu przeładowania rejestru przez backend pokazujemy je lokalnie.
  const added = useRef(new Map<string, { at: number; action: ActionDef | null }>());
  const seq = useRef(0);
  const formRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    try {
      const [a, m] = await Promise.all([api.GET("/actions"), api.GET("/actions/missing")]);
      if (a.error || !a.data || m.error || !m.data) throw new Error("actions");
      setLoadError(null);
      const now = Date.now();
      const fresh = [...added.current.values()].filter((x) => x.action && now - x.at <= ADDED_HIDE_MS);
      const have = new Set(a.data.map((x) => x.name));
      const merged = [...a.data, ...fresh.map((x) => x.action as ActionDef).filter((x) => !have.has(x.name))];
      setActions(merged.sort((x, y) => x.name.localeCompare(y.name)));
      setMissing(m.data.filter((x) => now - (added.current.get(x.name)?.at ?? -Infinity) > ADDED_HIDE_MS));
    } catch {
      setLoadError("Nie udało się pobrać rejestru akcji.");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Na żywo: zmiana konfiguracji (np. nowa akcja z pliku YAML) odświeża listy.
  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "config_changed") void load();
  });

  function openForm(target: Omit<FormTarget, "key">) {
    setNotice(null);
    setForm({ key: ++seq.current, ...target });
  }

  function onSaved(saved: ActionDef) {
    added.current.set(saved.name, { at: Date.now(), action: saved });
    setActions((cur) => {
      const rest = (cur ?? []).filter((a) => a.name !== saved.name);
      return [...rest, saved].sort((x, y) => x.name.localeCompare(y.name));
    });
    setMissing((cur) => cur?.filter((m) => m.name !== saved.name) ?? cur);
    setForm(null);
    setNotice(`Zapisano akcję ${saved.name}. Zmiana jest widoczna w rejestrze w ciągu kilku sekund.`);
    void load();
  }

  async function remove(name: string) {
    setDeleting(true);
    setDeleteError(null);
    try {
      const { response } = await api.DELETE("/actions/{name}", { params: { path: { name } } });
      if (!response.ok) throw new Error(String(response.status));
      added.current.delete(name);
      setActions((cur) => cur?.filter((a) => a.name !== name) ?? cur);
      setConfirmDelete(null);
      setForm((cur) => (cur && !cur.isNew && cur.initial.name === name ? null : cur));
      setNotice(`Usunięto akcję ${name}. Kroki runbooków, które jej używały, staną się ręczne.`);
      void load();
    } catch {
      setDeleteError(`Nie udało się usunąć akcji ${name}.`);
    } finally {
      setDeleting(false);
    }
  }

  const takenNames = new Set((actions ?? []).map((a) => a.name));
  const visibleMissing = missing?.filter((m) => !takenNames.has(m.name)) ?? null;

  return (
    <section className="actions">
      <header className="act-head">
        <h1>Akcje</h1>
        <div className="act-head-btns">
          <button type="button" onClick={() => void load()}>
            Odśwież
          </button>
          <button type="button" className="primary" onClick={() => openForm({ initial: emptyForm(), isNew: true })}>
            Nowa akcja HTTP
          </button>
        </div>
      </header>
      <p className="muted">
        Rejestr akcji, które mogą wykonywać runbooki. Wykonywane są wyłącznie akcje z rejestru; akcje o wysokim ryzyku
        zawsze wymagają zatwierdzenia w UI.
      </p>

      {loadError && (
        <p className="error-box" role="alert">
          {loadError}
        </p>
      )}
      {notice && (
        <p className="act-notice" role="status">
          {notice}
        </p>
      )}

      {form && (
        <div ref={formRef} className="act-form-wrap">
          <ActionForm
            key={form.key}
            initial={form.initial}
            isNew={form.isNew}
            version={form.version}
            takenNames={takenNames}
            onSaved={onSaved}
            onCancel={() => setForm(null)}
          />
        </div>
      )}

      <section aria-labelledby="act-missing-title" className="act-block">
        <h2 id="act-missing-title">Brakujące akcje</h2>
        <p className="muted">
          Akcje użyte w runbookach, których nie ma w rejestrze. Takie kroki są dziś wykonywane ręcznie — po dodaniu akcji
          staną się automatyczne.
        </p>
        {visibleMissing === null && !loadError && <p className="loading">Ładowanie…</p>}
        {visibleMissing?.length === 0 && <p className="empty">Brak brakujących akcji.</p>}
        {visibleMissing && visibleMissing.length > 0 && (
          <table className="act-table">
            <caption className="sr-only">Akcje użyte w runbookach, ale nieobecne w rejestrze</caption>
            <thead>
              <tr>
                <th scope="col">Akcja</th>
                <th scope="col">Użyta w runbookach</th>
                <th scope="col" className="num">
                  Wykonań ręcznych
                </th>
                <th scope="col">
                  <span className="sr-only">Operacje</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {visibleMissing.map((m) => (
                <tr key={m.name}>
                  <th scope="row" className="mono">
                    {m.name}
                  </th>
                  <td>
                    <span className="act-count">{m.runbook_ids.length}</span>{" "}
                    {m.runbook_ids.map((id, i) => (
                      <span key={id}>
                        {i > 0 && ", "}
                        <Link to="/runbooks" className="mono" title="Przejdź do runbooków">
                          {id}
                        </Link>
                      </span>
                    ))}
                  </td>
                  <td className="num">{m.manual_executions}</td>
                  <td className="act-row-btns">
                    <button
                      type="button"
                      aria-label={`Dodaj akcję ${m.name}`}
                      onClick={() => openForm({ initial: emptyForm(m.name), isNew: true })}
                    >
                      Dodaj
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section aria-labelledby="act-registry-title" className="act-block">
        <h2 id="act-registry-title">Rejestr akcji</h2>
        {actions === null && !loadError && <p className="loading">Ładowanie…</p>}
        {actions?.length === 0 && <p className="empty">Rejestr jest pusty.</p>}
        {deleteError && (
          <p className="error-box" role="alert">
            {deleteError}
          </p>
        )}
        {actions && actions.length > 0 && (
          <table className="act-table">
            <caption className="sr-only">Zarejestrowane akcje</caption>
            <thead>
              <tr>
                <th scope="col">Nazwa</th>
                <th scope="col">Rodzaj</th>
                <th scope="col">Ryzyko</th>
                <th scope="col">Opis</th>
                <th scope="col" className="num">
                  Wersja
                </th>
                <th scope="col">
                  <span className="sr-only">Operacje</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {actions.map((a) => {
                const http = a.kind === "http";
                const confirming = confirmDelete === a.name;
                return (
                  <tr key={a.name} className={a.risk === "high" ? "act-high" : undefined}>
                    <th scope="row" className="mono">
                      {a.name}
                    </th>
                    <td>
                      {a.kind === "builtin" ? "wbudowana" : "HTTP"}
                      {http && a.http && (
                        <div className="act-endpoint mono">
                          {a.http.method} {a.http.url}
                        </div>
                      )}
                    </td>
                    <td>
                      <RiskBadge risk={a.risk} />
                    </td>
                    <td>{a.description ?? <span className="muted">—</span>}</td>
                    <td className="num">{a.version ?? 1}</td>
                    <td className="act-row-btns">
                      {!http && <span className="muted">tylko do odczytu</span>}
                      {http && !confirming && (
                        <>
                          <button
                            type="button"
                            aria-label={`Edytuj akcję ${a.name}`}
                            onClick={() => openForm({ initial: fromAction(a), isNew: false, version: a.version })}
                          >
                            Edytuj
                          </button>
                          <button
                            type="button"
                            className="danger"
                            aria-label={`Usuń akcję ${a.name}`}
                            onClick={() => {
                              setDeleteError(null);
                              setConfirmDelete(a.name);
                            }}
                          >
                            Usuń
                          </button>
                        </>
                      )}
                      {http && confirming && (
                        <>
                          <span className="act-confirm">Usunąć?</span>
                          <button type="button" className="danger" disabled={deleting} onClick={() => void remove(a.name)}>
                            {deleting ? "Usuwanie…" : "Tak, usuń"}
                          </button>
                          <button type="button" disabled={deleting} onClick={() => setConfirmDelete(null)}>
                            Anuluj
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
    </section>
  );
}
