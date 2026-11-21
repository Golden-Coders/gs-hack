// Mappingi i zdarzenia UNMAPPED (tasks.md 15.2): lista schematów bez mappingu z przykładowym payloadem
// oraz istniejących mappingów; tworzenie i edycja mappingu przez wybór pól z przykładowego payloadu.
import { useCallback, useEffect, useRef, useState } from "react";
import Button from "@atlaskit/button/new";
import { Code, CodeBlock } from "@atlaskit/code";
import DynamicTable from "@atlaskit/dynamic-table";
import { useFlags } from "@atlaskit/flag/use-flags";
import { Inline, Text } from "@atlaskit/primitives/compiled";
import { useNavigate } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Card, ConfirmInline, Empty, ErrorBox, Field, Loading, PageHeader, TextInput } from "../ui/primitives";
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
      <CodeBlock language={p.format === "yaml" ? "yaml" : "json"} text={text} />
    </details>
  );
}

export default function Mappings() {
  const unmappedRes = useResource(() => api.GET("/unmapped"));
  const mappingsRes = useResource(() => api.GET("/mappings"));
  const unmapped: Unmapped[] = unmappedRes.data ?? [];
  const mappings: Mapping[] = mappingsRes.data ?? [];
  const [editing, setEditing] = useState<Editing | null>(null);
  const { showFlag } = useFlags();
  const navigate = useNavigate();
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
    setEditing({ mapping: newMapping(u.schema), sample: u.sample, count: u.count, isNew: true });
  }

  function startEdit(m: Mapping) {
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
    setEditing({ mapping: newMapping(schema), sample: u?.sample, count: u?.count, isNew: true });
  }

  function saved(m: Mapping) {
    setEditing(null);
    showFlag({
      id: `mapping-saved-${m.schema}`,
      appearance: "success",
      title: `Zapisano mapping ${m.schema}${m.version && m.version > 0 ? ` (wersja ${m.version})` : ""}`,
      description: "Czekające zdarzenia są przetwarzane ponownie.",
      isAutoDismiss: true,
      actions: [{ content: "Zobacz alerty w feedzie", onClick: () => navigate("/alerts") }],
    });
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
      <PageHeader actions={<Button onClick={refresh}>Odśwież</Button>}>Mappingi i UNMAPPED</PageHeader>
      <Text as="p" color="color.text.subtle">
        Zdarzenie ze schematem bez mappingu nie ginie: czeka jako UNMAPPED. Utwórz mapping, wskazując pola w
        przykładowym payloadzie – zdarzenia zostaną przetworzone i pojawią się w feedzie.
      </Text>

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
                  <Button appearance="primary" onClick={() => startNew(u)}>
                    Utwórz mapping
                  </Button>
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
                <TextInput
                  {...field}
                  className="mono"
                  value={manual}
                  onChange={(e) => setManual(e.currentTarget.value)}
                  placeholder="np. judges.custom.v1"
                />
                <Button type="submit">Utwórz</Button>
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
          <DynamicTable
            head={{
              cells: [
                { key: "schema", content: "Schemat" },
                { key: "severity", content: "Severity" },
                { key: "fingerprint", content: "Fingerprint" },
                { key: "resolved", content: "Resolved" },
                { key: "version", content: "Wersja" },
                { key: "actions", content: "Akcje" },
              ],
            }}
            rows={mappings.map((m) => ({
              key: m.schema,
              cells: [
                { key: "schema", content: <span className="mono">{m.schema}</span> },
                { key: "severity", content: severitySummary(m) },
                {
                  key: "fingerprint",
                  content: m.fingerprint?.length ? <Code>{m.fingerprint.join(", ")}</Code> : <Text color="color.text.subtle">cały payload</Text>,
                },
                { key: "resolved", content: m.resolved ? "tak" : <Text color="color.text.subtle">brak</Text> },
                { key: "version", content: m.version ?? "—" },
                {
                  key: "actions",
                  content: (
                    <Inline space="space.100" alignBlock="center" alignInline="end" shouldWrap>
                      <Button onClick={() => startEdit(m)} aria-label={`Edytuj mapping ${m.schema}`}>
                        Edytuj
                      </Button>
                      <ConfirmInline
                        label={<>Usuń<span className="sr-only"> mapping {m.schema}</span></>}
                        question="Usunąć? Nowe zdarzenia tego schematu trafią do UNMAPPED."
                        confirmLabel="Tak, usuń"
                        busy={deleting}
                        onConfirm={() => void remove(m.schema)}
                      />
                    </Inline>
                  ),
                },
              ],
            }))}
          />
        )}
      </Card>
    </section>
  );
}
