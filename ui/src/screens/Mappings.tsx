// Mappingi i zdarzenia UNMAPPED (tasks.md 15.2): lista schematów bez mappingu z przykładowym payloadem
// oraz istniejących mappingów; tworzenie i edycja mappingu przez wybór pól z przykładowego payloadu.
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Card, ConfirmInline, Empty, ErrorBox, Field, Loading } from "../ui/primitives";
import { formatTime } from "../ui/format";
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
  const unmappedRes = useResource(() => api.GET("/unmapped"));
  const mappingsRes = useResource(() => api.GET("/mappings"));
  const unmapped: Unmapped[] = unmappedRes.data ?? [];
  const mappings: Mapping[] = mappingsRes.data ?? [];
  const [editing, setEditing] = useState<Editing | null>(null);
  const [notice, setNotice] = useState<{ schema: string; version?: number } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [manual, setManual] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const timers = useRef<number[]>([]);

  // Ciche odświeżenie obu list (bez wskaźnika ładowania).
  const { refresh: refreshUnmapped } = unmappedRes;
  const { refresh: refreshMappings } = mappingsRes;
  const refresh = useCallback(() => {
    refreshUnmapped();
    refreshMappings();
  }, [refreshUnmapped, refreshMappings]);

  useEffect(() => {
    const pending = timers.current;
    return () => pending.forEach((t) => window.clearTimeout(t));
  }, []);

  // Nowe UNMAPPED i zmiany konfiguracji przychodzą na żywo (na mocku SSE jest wyłączone – służy przycisk „Odśwież”).
  useStream((msg) => {
    if (msg.type === "ui_event" && (msg.data.kind === "unmapped" || msg.data.kind === "config_changed")) refresh();
  });
  usePollWhenQuiet(refresh);

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
    refresh();
    // Ponowne przetworzenie UNMAPPED dzieje się asynchronicznie – odświeżamy listę jeszcze dwa razy.
    timers.current.push(window.setTimeout(refresh, 2_000), window.setTimeout(refresh, 5_000));
  }

  async function remove(schema: string) {
    setDeleting(true);
    setDeleteError(null);
    const failure = await send(() => api.DELETE("/mappings/{schema}", { params: { path: { schema } } }));
    if (failure) setDeleteError(`Nie udało się usunąć mappingu ${schema}: ${failure}`);
    else refresh();
    setDeleting(false);
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
        <button type="button" onClick={refresh}>
          Odśwież
        </button>
      </header>

      {notice && (
        <p className="mp-success" role="status">
          Zapisano mapping <strong className="mono">{notice.schema}</strong>
          {notice.version && notice.version > 0 ? ` (wersja ${notice.version})` : ""}. Czekające zdarzenia są przetwarzane ponownie –{" "}
          <Link to="/alerts">zobacz alerty w feedzie</Link>.
        </p>
      )}

      <Card title="Krok 1. Wybierz schemat bez mappingu (UNMAPPED)" error={unmappedRes.error} onRetry={unmappedRes.reload} loading={unmappedRes.loading && unmappedRes.data !== null}>
        {unmappedRes.loading && unmappedRes.data === null ? (
          <Loading />
        ) : unmapped.length === 0 ? (
          <Empty>Brak zdarzeń UNMAPPED. Gdy przyjdzie zdarzenie nieznanego schematu, pojawi się tutaj.</Empty>
        ) : (
          <ul className="mp-cards">
            {unmapped.map((u) => (
              <li key={u.schema} className="mp-card">
                <div className="mp-card-top">
                  <div>
                    <div className="mono mp-schema">{u.schema}</div>
                    <div className="muted mp-small">
                      {u.count} {u.count === 1 ? "zdarzenie" : "zdarzeń"} · ostatnie {formatTime(u.last_seen)}
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
          <Field label="Albo nowy schemat ręcznie (wkleisz własny payload)" error={manualError}>
            {(field) => (
              <div className="mp-inline">
                <input
                  {...field}
                  className="mono"
                  value={manual}
                  onChange={(e) => setManual(e.target.value)}
                  placeholder="np. judges.custom.v1"
                />
                <button type="submit">Utwórz</button>
              </div>
            )}
          </Field>
        </form>
      </Card>

      <Card title="Istniejące mappingi" error={mappingsRes.error} onRetry={mappingsRes.reload} loading={mappingsRes.loading && mappingsRes.data !== null}>
        {deleteError && <ErrorBox message={deleteError} />}
        {mappingsRes.loading && mappingsRes.data === null ? (
          <Loading />
        ) : mappings.length === 0 ? (
          <Empty>Brak mappingów.</Empty>
        ) : (
          <div className="table-wrap">
            <table className="data-table">
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
                      <div className="mp-row-actions">
                        <button type="button" onClick={() => startEdit(m)} aria-label={`Edytuj mapping ${m.schema}`}>
                          Edytuj
                        </button>
                        <ConfirmInline
                          label={<>Usuń<span className="sr-only"> mapping {m.schema}</span></>}
                          question="Usunąć? Nowe zdarzenia tego schematu trafią do UNMAPPED."
                          confirmLabel="Tak, usuń"
                          busy={deleting}
                          onConfirm={() => void remove(m.schema)}
                        />
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </section>
  );
}
