import { useCallback, useEffect, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Card, Empty, ErrorBox, Loading, PageHeader } from "../ui/primitives";
import MappingEditor from "./mappings/MappingEditor";
import "./Mappings.css";

type Mapping = Schemas["MappingDef"];
type Unmapped = Schemas["UnmappedSchema"];

const SEVERITY_PL: Record<string, string> = {
  critical: "Krytyczny",
  error: "Błąd",
  warning: "Ostrzeżenie",
  info: "Informacja",
};

function newMapping(schema: string): Mapping {
  return {
    schema,
    fields: {},
    title: null,
    severity: { type: "const", value: "warning" },
    fingerprint: [],
    resolved: null,
  };
}

function cleanPath(p: string | null | undefined): string {
  if (!p) return "Domyślny (nazwa schematu)";
  return p.startsWith("$.") ? p.slice(2) : p;
}

function severitySummary(m: Mapping): string {
  const s = m.severity;
  if (s.type === "const") return `Stały: ${SEVERITY_PL[s.value ?? "warning"] ?? s.value}`;
  if (s.type === "path") return `Z pola: ${cleanPath(s.path)}`;
  return `Według warunku (${s.rules?.length ?? 0})`;
}

export default function Mappings() {
  const unmappedRes = useResource(() => api.GET("/unmapped"));
  const mappingsRes = useResource(() => api.GET("/mappings"));
  const unmapped: Unmapped[] = unmappedRes.data ?? [];
  const mappings: Mapping[] = mappingsRes.data ?? [];

  const [selectedSchema, setSelectedSchema] = useState<string | null>(null);
  const [isCreatingNew, setIsCreatingNew] = useState(false);
  const [customSample, setCustomSample] = useState<unknown>(undefined);

  const { refresh: refreshUnmapped } = unmappedRes;
  const { refresh: refreshMappings } = mappingsRes;
  const refresh = useCallback(() => {
    refreshUnmapped();
    refreshMappings();
  }, [refreshUnmapped, refreshMappings]);

  useStream((msg) => {
    if (msg.type === "ui_event" && (msg.data.kind === "unmapped" || msg.data.kind === "config_changed")) {
      refresh();
    }
  });
  usePollWhenQuiet(refresh);

  useEffect(() => {
    if (selectedSchema !== null || isCreatingNew) return;
    if (mappings.length > 0) {
      setSelectedSchema(mappings[0].schema);
    } else if (unmapped.length > 0) {
      setSelectedSchema(unmapped[0].schema);
    }
  }, [mappings, unmapped, selectedSchema, isCreatingNew]);

  const activeMapping: Mapping | null = isCreatingNew
    ? newMapping("")
    : selectedSchema
      ? (mappings.find((m) => m.schema === selectedSchema) ?? newMapping(selectedSchema))
      : null;

  const activeIsNew = isCreatingNew || (selectedSchema !== null && !mappings.some((m) => m.schema === selectedSchema));
  const activeSample =
    customSample ?? unmapped.find((u) => u.schema === selectedSchema)?.sample;

  function handleSelectExisting(m: Mapping) {
    setIsCreatingNew(false);
    setCustomSample(undefined);
    setSelectedSchema(m.schema);
  }

  function handleSelectUnmapped(u: Unmapped) {
    setIsCreatingNew(false);
    setCustomSample(u.sample);
    setSelectedSchema(u.schema);
  }

  function handleCreateBlank() {
    setSelectedSchema(null);
    setCustomSample(undefined);
    setIsCreatingNew(true);
  }

  function handleSaved(saved: Mapping) {
    setIsCreatingNew(false);
    setSelectedSchema(saved.schema);
    refresh();
  }

  async function handleDelete(schema: string) {
    const err = await send(() => api.DELETE("/mappings/{schema}", { params: { path: { schema } } }));
    if (!err) {
      const remaining = mappings.filter((m) => m.schema !== schema);
      setSelectedSchema(remaining[0]?.schema ?? null);
      setIsCreatingNew(false);
      refresh();
    }
    return err;
  }

  return (
    <section className="mp-screen">
      <PageHeader
        actions={
          <Button appearance="primary" onClick={handleCreateBlank}>
            + Nowy mapping
          </Button>
        }
      >
        Mappingi
      </PageHeader>

      {mappingsRes.error && <ErrorBox message={mappingsRes.error} onRetry={refresh} />}

      {/* Baner z nowymi nieobsłużonymi typami zdarzeń (tylko gdy faktycznie jakieś czekają) */}
      {unmapped.length > 0 && (
        <Card title={`Nowe nieobsłużone typy zdarzeń (${unmapped.length})`}>
          <ul className="mp-unmapped-list">
            {unmapped.map((u) => (
              <li key={u.schema} className="mp-unmapped-item">
                <div>
                  <Text weight="semibold">{u.schema}</Text>
                  <div className="muted mp-small">
                    Oczekujących zdarzeń: {u.count}
                  </div>
                </div>
                <Button appearance="primary" onClick={() => handleSelectUnmapped(u)}>
                  Skonfiguruj
                </Button>
              </li>
            ))}
          </ul>
        </Card>
      )}

      <div className="mp-layout">
        {/* Lewa kolumna: lista skonfigurowanych schematów */}
        <aside className="mp-sidebar" aria-label="Lista mappingów">
          {mappingsRes.loading && mappingsRes.data === null && <Loading />}
          {!mappingsRes.loading && mappings.length === 0 && (
            <Empty>Brak mappingów. Kliknij „+ Nowy mapping”.</Empty>
          )}
          <ul className="mp-list">
            {mappings.map((m) => {
              const active = !isCreatingNew && m.schema === selectedSchema;
              return (
                <li key={m.schema}>
                  <button
                    type="button"
                    className={`mp-item ${active ? "active" : ""}`}
                    onClick={() => handleSelectExisting(m)}
                  >
                    <div className="mp-item-title">{m.schema}</div>
                    <div className="mp-item-sub">
                      Tytuł: <strong>{cleanPath(m.title)}</strong>
                    </div>
                    <div className="mp-item-sub">{severitySummary(m)}</div>
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>

        {/* Prawa kolumna: prosty edytor wybranego mappingu */}
        <div className="mp-main">
          {activeMapping ? (
            <MappingEditor
              key={isCreatingNew ? "__new__" : activeMapping.schema}
              initial={activeMapping}
              sample={activeSample}
              isNew={activeIsNew}
              onSaved={handleSaved}
              onDelete={handleDelete}
            />
          ) : (
            <Card>
              <Heading size="small" as="h2">
                Wybierz mapping z listy po lewej stronie
              </Heading>
            </Card>
          )}
        </div>
      </div>
    </section>
  );
}
