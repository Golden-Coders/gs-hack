// Mappingi i zdarzenia UNMAPPED (tasks.md 15.2): lista schematów bez mappingu z przykładowym payloadem
// oraz istniejących mappingów; tworzenie i edycja mappingu przez wybór pól z przykładowego payloadu.
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { useStream } from "../api/stream";
import MappingEditor from "./mappings/MappingEditor";
import { parseSample } from "./mappings/sample";
import "./Mappings.css";

type Mapping = Schemas["MappingDef"];
type Unmapped = Schemas["UnmappedSchema"];

type Editing = { mapping: Mapping; sample: unknown; count?: number; isNew: boolean };

function newMapping(schema: string): Mapping {
  return { schema, fields: {}, title: null, severity: { type: "const", value: "warning" }, fingerprint: [], resolved: null };
}

function severitySummary(m: Mapping): string {
  const s = m.severity;
  if (s.type === "const") return `stała: ${s.value}`;
  if (s.type === "path") return `z pola ${s.path}`;
  return `z warunków (${s.rules?.length ?? 0})`;
}

function SampleView({ sample }: { sample: unknown }) {
  const p = parseSample(sample);
  const text = typeof sample === "string" ? sample : JSON.stringify(sample, null, 2);
  return (
    <details className="mp-sample-view">
      <summary>Przykładowy payload{p.format !== "invalid" ? ` (${p.format.toUpperCase()})` : ""}</summary>
      <pre className="mono">{text}</pre>
    </details>
  );
}

export default function Mappings() {
  const [unmapped, setUnmapped] = useState<Unmapped[]>([]);
  const [mappings, setMappings] = useState<Mapping[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [notice, setNotice] = useState<{ schema: string; version?: number } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [manual, setManual] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const timers = useRef<number[]>([]);

  const load = useCallback(async () => {
    try {
      const [u, m] = await Promise.all([api.GET("/unmapped"), api.GET("/mappings")]);
      if (u.data) setUnmapped(u.data);
      if (m.data) setMappings(m.data);
      const failed = !u.data ? u : !m.data ? m : null;
      setError(failed ? `Nie udało się pobrać danych (${failed.response.status}).` : null);
    } catch (e) {
      setError(`Brak połączenia z serwerem: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const pending = timers.current;
    return () => pending.forEach((t) => window.clearTimeout(t));
  }, [load]);

  // Nowe UNMAPPED i zmiany konfiguracji przychodzą na żywo (na mocku SSE jest wyłączone – służy przycisk „Odśwież”).
  useStream((msg) => {
    if (msg.type === "ui_event" && (msg.data.kind === "unmapped" || msg.data.kind === "config_changed")) void load();
  });

  function startNew(u: Unmapped) {
    setNotice(null);
    setEditing({ mapping: newMapping(u.schema), sample: u.sample, count: u.count, isNew: true });
  }

  function startEdit(m: Mapping) {
    setNotice(null);
    const u = unmapped.find((x) => x.schema === m.schema);
    setEditing({ mapping: m, sample: u?.sample, count: u?.count, isNew: false });
  }

  function startManual(e: React.FormEvent) {
    e.preventDefault();
    const schema = manual.trim();
    if (!schema) return setManualError("Podaj nazwę schematu.");
    const existing = mappings.find((m) => m.schema === schema);
    if (existing) return startEdit(existing);
    const u = unmapped.find((x) => x.schema === schema);
    setManualError(null);
    setManual("");
    setNotice(null);
    setEditing({ mapping: newMapping(schema), sample: u?.sample, count: u?.count, isNew: true });
  }

  function saved(m: Mapping) {
    setEditing(null);
    setNotice({ schema: m.schema, version: m.version });
    void load();
    // Ponowne przetworzenie UNMAPPED dzieje się asynchronicznie – odświeżamy listę jeszcze dwa razy.
    timers.current.push(window.setTimeout(() => void load(), 2_000), window.setTimeout(() => void load(), 5_000));
  }

  async function remove(schema: string) {
    setDeleting(true);
    setDeleteError(null);
    try {
      const { response } = await api.DELETE("/mappings/{schema}", { params: { path: { schema } } });
      if (response.ok) {
        setConfirmDelete(null);
        await load();
      } else setDeleteError(`Nie udało się usunąć mappingu ${schema} (${response.status}).`);
    } catch (e) {
      setDeleteError(`Brak połączenia z serwerem: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setDeleting(false);
    }
  }

  if (editing) {
    return (
      <section className="mp">
        <MappingEditor
          key={`${editing.mapping.schema}:${editing.isNew}`}
          initial={editing.mapping}
          sample={editing.sample}
          eventCount={editing.count}
          isNew={editing.isNew}
          onSaved={saved}
          onClose={() => setEditing(null)}
        />
      </section>
    );
  }

  return (
    <section className="mp">
      <header className="mp-head">
        <div>
          <h1>Mappingi i UNMAPPED</h1>
          <p className="muted">
            Zdarzenie ze schematem bez mappingu nie ginie: czeka jako UNMAPPED. Utwórz mapping, wskazując pola w
            przykładowym payloadzie – zdarzenia zostaną przetworzone i pojawią się w feedzie.
          </p>
        </div>
        <button type="button" onClick={() => void load()}>
          Odśwież
        </button>
      </header>

      {error && <p className="error-box" role="alert">{error}</p>}
      {notice && (
        <p className="mp-success" role="status">
          Zapisano mapping <strong className="mono">{notice.schema}</strong>
          {notice.version && notice.version > 0 ? ` (wersja ${notice.version})` : ""}. Czekające zdarzenia są przetwarzane ponownie –{" "}
          <Link to="/alerts">zobacz alerty w feedzie</Link>.
        </p>
      )}

      <section className="mp-block" aria-labelledby="mp-unmapped-h">
        <h2 id="mp-unmapped-h">Krok 1. Wybierz schemat bez mappingu (UNMAPPED)</h2>
        {loading ? (
          <p className="loading">Ładowanie…</p>
        ) : unmapped.length === 0 ? (
          <p className="empty">Brak zdarzeń UNMAPPED. Gdy przyjdzie zdarzenie nieznanego schematu, pojawi się tutaj.</p>
        ) : (
          <ul className="mp-cards">
            {unmapped.map((u) => (
              <li key={u.schema} className="mp-card">
                <div className="mp-card-top">
                  <div>
                    <div className="mono mp-schema">{u.schema}</div>
                    <div className="muted mp-small">
                      {u.count} {u.count === 1 ? "zdarzenie" : "zdarzeń"} · ostatnie {new Date(u.last_seen).toLocaleString()}
                    </div>
                  </div>
                  <button type="button" className="primary" onClick={() => startNew(u)}>
                    Utwórz mapping
                  </button>
                </div>
                <SampleView sample={u.sample} />
              </li>
            ))}
          </ul>
        )}
        <form className="mp-manual" onSubmit={startManual} noValidate>
          <div className="mp-lbl">
            <label htmlFor="mp-manual">Albo nowy schemat ręcznie (wkleisz własny payload)</label>
            <div className="mp-inline">
              <input
                id="mp-manual"
                className="mono"
                value={manual}
                onChange={(e) => setManual(e.target.value)}
                placeholder="np. judges.custom.v1"
                aria-invalid={manualError ? true : undefined}
              />
              <button type="submit">Utwórz</button>
            </div>
          </div>
          {manualError && <p className="mp-err" role="alert">{manualError}</p>}
        </form>
      </section>

      <section className="mp-block" aria-labelledby="mp-existing-h">
        <h2 id="mp-existing-h">Istniejące mappingi</h2>
        {deleteError && <p className="error-box" role="alert">{deleteError}</p>}
        {loading ? (
          <p className="loading">Ładowanie…</p>
        ) : mappings.length === 0 ? (
          <p className="empty">Brak mappingów.</p>
        ) : (
          <div className="mp-table-wrap">
            <table className="mp-table">
              <thead>
                <tr>
                  <th scope="col">Schemat</th>
                  <th scope="col">Severity</th>
                  <th scope="col">Fingerprint</th>
                  <th scope="col">Resolved</th>
                  <th scope="col">Wersja</th>
                  <th scope="col"><span className="sr-only">Akcje</span></th>
                </tr>
              </thead>
              <tbody>
                {mappings.map((m) => (
                  <tr key={m.schema}>
                    <td className="mono">{m.schema}</td>
                    <td>{severitySummary(m)}</td>
                    <td className="mono mp-small">
                      {m.fingerprint?.length ? m.fingerprint.join(", ") : <span className="muted">cały payload</span>}
                    </td>
                    <td>{m.resolved ? "tak" : <span className="muted">brak</span>}</td>
                    <td>{m.version ?? "—"}</td>
                    <td className="mp-actions-cell">
                      {confirmDelete === m.schema ? (
                        <span className="mp-confirm" role="group" aria-label={`Potwierdź usunięcie ${m.schema}`}>
                          <span>Usunąć? Nowe zdarzenia tego schematu trafią do UNMAPPED.</span>
                          <button type="button" className="danger" disabled={deleting} onClick={() => void remove(m.schema)}>
                            {deleting ? "Usuwanie…" : "Tak, usuń"}
                          </button>
                          <button type="button" disabled={deleting} onClick={() => setConfirmDelete(null)}>
                            Anuluj
                          </button>
                        </span>
                      ) : (
                        <>
                          <button type="button" onClick={() => startEdit(m)} aria-label={`Edytuj mapping ${m.schema}`}>
                            Edytuj
                          </button>
                          <button
                            type="button"
                            className="danger"
                            onClick={() => {
                              setDeleteError(null);
                              setConfirmDelete(m.schema);
                            }}
                            aria-label={`Usuń mapping ${m.schema}`}
                          >
                            Usuń
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </section>
  );
}
