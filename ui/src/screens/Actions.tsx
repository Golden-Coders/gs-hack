// Akcje — tasks.md 15.3: rejestr akcji (builtin tylko do odczytu), formularz akcji HTTP
// oraz lista akcji brakujących w rejestrze z przyciskiem „Dodaj”.
import { useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { RiskBadge } from "../components/Badges";
import { Card, ConfirmInline, Empty, ErrorBox, Loading } from "../ui/primitives";
import ActionForm from "./actions/ActionForm";
import { emptyForm, fromAction, type ActionDef, type FormState } from "./actions/model";
import "./Actions.css";

type Missing = Schemas["MissingAction"];

type FormTarget = { key: number; initial: FormState; isNew: boolean; version?: number };

// Rejestr przeładowuje się po NOTIFY w ciągu kilku sekund – świeżo dodaną nazwę ukrywamy na krótko lokalnie.
const ADDED_HIDE_MS = 15_000;

export default function Actions() {
  const [form, setForm] = useState<FormTarget | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Świeżo zapisane akcje: do czasu przeładowania rejestru przez backend pokazujemy je lokalnie.
  const added = useRef(new Map<string, { at: number; action: ActionDef | null }>());
  const seq = useRef(0);
  const formRef = useRef<HTMLDivElement>(null);

  const resource = useResource(async () => {
    const [a, m] = await Promise.all([api.GET("/actions"), api.GET("/actions/missing")]);
    if (a.data === undefined) return { error: a.error, response: a.response };
    if (m.data === undefined) return { error: m.error, response: m.response };
    const now = Date.now();
    const fresh = [...added.current.values()].filter((x) => x.action && now - x.at <= ADDED_HIDE_MS);
    const have = new Set(a.data.map((x) => x.name));
    const merged = [...a.data, ...fresh.map((x) => x.action as ActionDef).filter((x) => !have.has(x.name))];
    return {
      data: {
        actions: merged.sort((x, y) => x.name.localeCompare(y.name)),
        missing: m.data.filter((x: Missing) => now - (added.current.get(x.name)?.at ?? -Infinity) > ADDED_HIDE_MS),
      },
      response: a.response,
    };
  });
  const actions = resource.data?.actions ?? null;
  const missing = resource.data?.missing ?? null;

  // Na żywo: zmiana konfiguracji (np. nowa akcja z pliku YAML) odświeża listy.
  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "config_changed") resource.refresh();
  });

  function openForm(target: Omit<FormTarget, "key">) {
    setNotice(null);
    setForm({ key: ++seq.current, ...target });
  }

  function onSaved(saved: ActionDef) {
    added.current.set(saved.name, { at: Date.now(), action: saved });
    setForm(null);
    setNotice(`Zapisano akcję ${saved.name}. Zmiana jest widoczna w rejestrze w ciągu kilku sekund.`);
    resource.refresh();
  }

  async function remove(name: string) {
    setDeleting(true);
    setDeleteError(null);
    const err = await send(() => api.DELETE("/actions/{name}", { params: { path: { name } } }));
    if (err) {
      setDeleteError(`Nie udało się usunąć akcji ${name}.`);
    } else {
      added.current.delete(name);
      setForm((cur) => (cur && !cur.isNew && cur.initial.name === name ? null : cur));
      setNotice(`Usunięto akcję ${name}. Kroki runbooków, które jej używały, staną się ręczne.`);
      resource.refresh();
    }
    setDeleting(false);
  }

  const takenNames = new Set((actions ?? []).map((a) => a.name));
  const visibleMissing = missing?.filter((m) => !takenNames.has(m.name)) ?? null;

  return (
    <section className="actions">
      <header className="act-head">
        <h1>Akcje</h1>
        <div className="act-head-btns">
          <button type="button" onClick={resource.reload}>
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

      {resource.error && <ErrorBox message={resource.error} onRetry={resource.reload} disabled={resource.loading} />}
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

      <Card title="Brakujące akcje" loading={resource.loading && missing !== null} className="act-block">
        <p className="muted">
          Akcje użyte w runbookach, których nie ma w rejestrze. Takie kroki są dziś wykonywane ręcznie — po dodaniu akcji
          staną się automatyczne.
        </p>
        {visibleMissing === null && !resource.error && <Loading />}
        {visibleMissing?.length === 0 && <Empty>Brak brakujących akcji.</Empty>}
        {visibleMissing && visibleMissing.length > 0 && (
          <div className="table-wrap">
            <table className="data-table">
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
          </div>
        )}
      </Card>

      <Card title="Rejestr akcji" loading={resource.loading && actions !== null} className="act-block">
        {actions === null && !resource.error && <Loading />}
        {actions?.length === 0 && <Empty>Rejestr jest pusty.</Empty>}
        {deleteError && <ErrorBox message={deleteError} />}
        {actions && actions.length > 0 && (
          <div className="table-wrap">
            <table className="data-table">
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
                        {http && (
                          <>
                            <button
                              type="button"
                              aria-label={`Edytuj akcję ${a.name}`}
                              onClick={() => openForm({ initial: fromAction(a), isNew: false, version: a.version })}
                            >
                              Edytuj
                            </button>
                            <ConfirmInline
                              label={
                                <>
                                  Usuń<span className="sr-only"> akcję {a.name}</span>
                                </>
                              }
                              question="Usunąć?"
                              confirmLabel="Tak, usuń"
                              busy={deleting}
                              onConfirm={() => void remove(a.name)}
                            />
                          </>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </section>
  );
}
