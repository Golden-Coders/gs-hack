import { useMemo, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../../api/client";
import { describeError } from "../../api/request";
import { Card, ConfirmInline, ErrorBox, Field, Segmented, Select, TextInput } from "../../ui/primitives";
import { parseSample } from "./sample";

type Mapping = Schemas["MappingDef"];
type Severity = Schemas["Severity"];

const SEVERITY_OPTIONS: { value: Severity; label: string }[] = [
  { value: "critical", label: "Krytyczny (critical)" },
  { value: "error", label: "Błąd (error)" },
  { value: "warning", label: "Ostrzeżenie (warning)" },
  { value: "info", label: "Informacja (info)" },
];

function stripJsonPath(p: string | null | undefined): string {
  if (!p) return "";
  const t = p.trim();
  return t.startsWith("$.") ? t.slice(2) : t === "$" ? "" : t;
}

function ensureJsonPath(p: string): string {
  const t = p.trim();
  if (!t) return "";
  if (t.startsWith("{{") || t.startsWith("$")) return t;
  return `$.${t.replace(/^\.+/, "")}`;
}

function collectSamplePaths(obj: unknown, prefix = "", out: { path: string; preview: string }[] = []): { path: string; preview: string }[] {
  if (obj === null || typeof obj !== "object" || Array.isArray(obj)) return out;
  for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
    const next = prefix ? `${prefix}.${k}` : k;
    if (v !== null && typeof v === "object" && !Array.isArray(v)) {
      collectSamplePaths(v, next, out);
    } else {
      const valStr = typeof v === "string" ? v : JSON.stringify(v);
      out.push({ path: next, preview: valStr ?? "" });
    }
  }
  return out;
}

interface FieldRow {
  uid: number;
  name: string;
  path: string;
}

let uidSeq = 1;

export default function MappingEditor({
  initial,
  sample,
  isNew,
  onSaved,
  onDelete,
}: {
  initial: Mapping;
  sample?: unknown;
  isNew: boolean;
  onSaved: (m: Mapping) => void;
  onDelete: (schema: string) => Promise<string | null>;
}) {
  const [schema, setSchema] = useState(initial.schema);
  const [title, setTitle] = useState(() => stripJsonPath(initial.title));

  const initialSevMode = initial.severity.type === "path" ? "path" : "const";
  const [sevMode, setSevMode] = useState<"const" | "path">(initialSevMode);
  const [sevConst, setSevConst] = useState<Severity>(
    initial.severity.type === "const"
      ? (initial.severity.value ?? "warning")
      : (initial.severity.default ?? "warning"),
  );
  const [sevPath, setSevPath] = useState<string>(
    initial.severity.type === "path" ? stripJsonPath(initial.severity.path) : "severity",
  );
  const [sevDefault, setSevDefault] = useState<Severity>(
    initial.severity.type !== "const" ? (initial.severity.default ?? "warning") : "warning",
  );

  const [groupFields, setGroupFields] = useState<string[]>(() => {
    const list = (initial.fingerprint ?? []).map(stripJsonPath).filter(Boolean);
    return list.length > 0 ? list : [""];
  });

  const [extraFields, setExtraFields] = useState<FieldRow[]>(() =>
    Object.entries(initial.fields ?? {}).map(([name, path]) => ({
      uid: uidSeq++,
      name,
      path: stripJsonPath(path),
    })),
  );

  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sampleFields = useMemo(() => {
    if (!sample) return [];
    const parsed = typeof sample === "string" ? parseSample(sample) : { format: "json" as const, value: sample };
    if (parsed.format === "invalid") return [];
    return collectSamplePaths(parsed.value);
  }, [sample]);

  function addGroupField(pathVal = "") {
    setGroupFields((prev) => {
      if (pathVal && prev.includes(pathVal)) return prev;
      if (prev.length === 1 && prev[0] === "" && pathVal) return [pathVal];
      return [...prev, pathVal];
    });
  }

  function updateGroupField(idx: number, val: string) {
    setGroupFields((prev) => prev.map((x, i) => (i === idx ? val : x)));
  }

  function removeGroupField(idx: number) {
    setGroupFields((prev) => (prev.length <= 1 ? [""] : prev.filter((_, i) => i !== idx)));
  }

  function addExtraField(pathVal = "") {
    const defaultName = pathVal ? (pathVal.split(".").pop() ?? pathVal) : "";
    setExtraFields((prev) => [...prev, { uid: uidSeq++, name: defaultName, path: pathVal }]);
  }

  function updateExtraField(uid: number, patch: Partial<FieldRow>) {
    setExtraFields((prev) => prev.map((r) => (r.uid === uid ? { ...r, ...patch } : r)));
  }

  function removeExtraField(uid: number) {
    setExtraFields((prev) => prev.filter((r) => r.uid !== uid));
  }

  async function handleSave() {
    setError(null);
    const cleanSchema = schema.trim();
    if (!cleanSchema) {
      setError("Podaj nazwę typu zdarzenia (schematu).");
      return;
    }
    if (sevMode === "path" && !sevPath.trim()) {
      setError("Podaj nazwę pola z priorytetem (np. severity lub labels.severity).");
      return;
    }

    const fieldsObj: Record<string, string> = {};
    for (const f of extraFields) {
      const n = f.name.trim();
      const p = f.path.trim();
      if (!n && !p) continue;
      if (!n || !p) {
        setError("W sekcji dodatkowych pól uzupełnij zarówno nazwę, jak i pole ze zdarzenia.");
        return;
      }
      fieldsObj[n] = ensureJsonPath(p);
    }

    const fingerprint = groupFields.map((g) => g.trim()).filter(Boolean).map(ensureJsonPath);

    const severity: Mapping["severity"] =
      sevMode === "const"
        ? { type: "const", value: sevConst }
        : {
            type: "path",
            path: ensureJsonPath(sevPath),
            map:
              initial.severity.type === "path" && initial.severity.map
                ? initial.severity.map
                : {
                    CRITICAL: "critical",
                    ERROR: "error",
                    WARN: "warning",
                    WARNING: "warning",
                    INFO: "info",
                  },
            default: sevDefault,
          };

    const body: Mapping = {
      schema: cleanSchema,
      title: title.trim() ? ensureJsonPath(title) : null,
      severity,
      fingerprint,
      fields: fieldsObj,
      resolved: initial.resolved ?? null,
    };

    setSaving(true);
    try {
      const { data, error: apiErr, response } = await api.PUT("/mappings/{schema}", {
        params: { path: { schema: cleanSchema } },
        body,
      });
      if (data) {
        onSaved({ ...data, schema: cleanSchema });
      } else {
        setError(describeError(apiErr, response));
      }
    } catch {
      setError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    setSaving(true);
    setError(null);
    const err = await onDelete(initial.schema);
    if (err) setError(`Nie udało się usunąć mappingu: ${err}`);
    setSaving(false);
  }

  return (
    <div className="mp-editor-clean">
      <header className="mp-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? (schema ? `Nowy mapping: ${schema}` : "Nowy mapping") : initial.schema}
        </Heading>
      </header>

      {/* Jeśli mamy przykładowe zdarzenie (np. z UNMAPPED), pokazujemy klikalne pola */}
      {sampleFields.length > 0 && (
        <Card title="Wykryte pola w przykładowym zdarzeniu (kliknij, aby użyć)">
          <div className="mp-sample-chips">
            {sampleFields.map((sf) => (
              <div key={sf.path} className="mp-chip-row">
                <code className="mp-chip-path">{sf.path}</code>
                <span className="mp-chip-val">{sf.preview}</span>
                <div className="mp-chip-btns">
                  <Button spacing="compact" onClick={() => setTitle(sf.path)}>
                    Jako tytuł
                  </Button>
                  <Button spacing="compact" onClick={() => addGroupField(sf.path)}>
                    Do grupowania
                  </Button>
                  <Button spacing="compact" onClick={() => addExtraField(sf.path)}>
                    Jako pole
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {/* 1. Podstawowe informacje: Typ zdarzenia i Tytuł */}
      <Card title="1. Typ zdarzenia i tytuł alertu">
        <div className="mp-two-col">
          <Field label="Nazwa typu zdarzenia (schema)">
            {(f) => (
              <TextInput
                {...f}
                value={schema}
                isDisabled={!isNew && Boolean(initial.schema)}
                onChange={(e) => setSchema(e.currentTarget.value)}
                placeholder="np. orders.failed.v1"
              />
            )}
          </Field>
          <Field
            label="Skąd brać tytuł alertu?"
            hint="Nazwa pola ze zdarzenia (np. error.message albo annotations.summary)"
          >
            {(f) => (
              <TextInput
                {...f}
                value={title}
                onChange={(e) => setTitle(e.currentTarget.value)}
                placeholder="np. error.message"
              />
            )}
          </Field>
        </div>
      </Card>

      {/* 2. Priorytet alertu */}
      <Card title="2. Jak ustalić priorytet alertu?">
        <div className="mp-sev-mode">
          <Segmented
            label="Sposób ustalania priorytetu"
            value={sevMode}
            onChange={(m) => setSevMode(m)}
            options={[
              { value: "const", label: "Zawsze taki sam priorytet" },
              { value: "path", label: "Odczytaj z pola w zdarzeniu" },
            ]}
          />
        </div>

        {sevMode === "const" ? (
          <Field label="Priorytet dla wszystkich zdarzeń tego typu">
            {(f) => (
              <Select
                {...f}
                value={sevConst}
                onChange={(v) => setSevConst(v as Severity)}
                options={SEVERITY_OPTIONS}
              />
            )}
          </Field>
        ) : (
          <div className="mp-two-col">
            <Field label="Pole z priorytetem w zdarzeniu" hint="np. severity lub labels.severity">
              {(f) => (
                <TextInput
                  {...f}
                  value={sevPath}
                  onChange={(e) => setSevPath(e.currentTarget.value)}
                  placeholder="np. severity"
                />
              )}
            </Field>
            <Field label="Domyślny priorytet (gdy pole jest puste)">
              {(f) => (
                <Select
                  {...f}
                  value={sevDefault}
                  onChange={(v) => setSevDefault(v as Severity)}
                  options={SEVERITY_OPTIONS}
                />
              )}
            </Field>
          </div>
        )}
      </Card>

      {/* 3. Grupowanie duplikatów */}
      <Card title="3. Po jakich polach łączyć powtarzające się alerty?">
        <Text as="p" size="small" color="color.text.subtle">
          Zdarzenia z taką samą wartością tych pól zostaną połączone w jeden alert zamiast tworzyć duplikaty.
        </Text>
        <div className="mp-field-list">
          {groupFields.map((g, idx) => (
            <div key={idx} className="mp-field-row">
              <div className="mp-grow">
                <Field label={idx === 0 ? "Pole grupujące" : "Kolejne pole"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={g}
                      onChange={(e) => updateGroupField(idx, e.currentTarget.value)}
                      placeholder="np. labels.service lub workflowName"
                    />
                  )}
                </Field>
              </div>
              <div className="mp-row-btn">
                <Button appearance="subtle" onClick={() => removeGroupField(idx)}>
                  Usuń
                </Button>
              </div>
            </div>
          ))}
        </div>
        <div className="mp-add-btn">
          <Button onClick={() => addGroupField("")}>+ Dodaj pole grupujące</Button>
        </div>
      </Card>

      {/* 4. Dodatkowe pola wyciągane do alertu i reguł */}
      <Card title="4. Dodatkowe pola widoczne w alercie i regułach (opcjonalnie)">
        {extraFields.length === 0 ? (
          <Text as="p" size="small" color="color.text.subtle">
            Brak dodatkowych pól. Dodaj pole, jeśli chcesz po nim filtrować w regułach (np. service, desk, region).
          </Text>
        ) : (
          <div className="mp-field-list">
            {extraFields.map((ef, idx) => (
              <div key={ef.uid} className="mp-extra-row">
                <Field label={idx === 0 ? "Krótka nazwa (np. service)" : "Krótka nazwa"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={ef.name}
                      onChange={(e) => updateExtraField(ef.uid, { name: e.currentTarget.value })}
                      placeholder="np. service"
                    />
                  )}
                </Field>
                <Field label={idx === 0 ? "Ścieżka w zdarzeniu (np. labels.service)" : "Ścieżka w zdarzeniu"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={ef.path}
                      onChange={(e) => updateExtraField(ef.uid, { path: e.currentTarget.value })}
                      placeholder="np. labels.service"
                    />
                  )}
                </Field>
                <div className="mp-row-btn">
                  <Button appearance="subtle" onClick={() => removeExtraField(ef.uid)}>
                    Usuń
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
        <div className="mp-add-btn">
          <Button onClick={() => addExtraField("")}>+ Dodaj pole</Button>
        </div>
      </Card>

      {error && <ErrorBox message={error} />}

      <div className="mp-savebar">
        <Button appearance="primary" isLoading={saving} isDisabled={saving} onClick={() => void handleSave()}>
          {isNew ? "Utwórz mapping" : "Zapisz zmiany"}
        </Button>
        {!isNew && (
          <ConfirmInline
            label="Usuń mapping"
            question={`Usunąć mapping „${initial.schema}”?`}
            confirmLabel="Tak, usuń"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </div>
  );
}
