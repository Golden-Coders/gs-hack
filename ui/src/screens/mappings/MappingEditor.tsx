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
  { value: "critical", label: "Critical" },
  { value: "error", label: "Error" },
  { value: "warning", label: "Warning" },
  { value: "info", label: "Info" },
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
      setError("Enter an event schema name.");
      return;
    }
    if (sevMode === "path" && !sevPath.trim()) {
      setError("Enter the field name containing severity (e.g. severity or labels.severity).");
      return;
    }

    const fieldsObj: Record<string, string> = {};
    for (const f of extraFields) {
      const n = f.name.trim();
      const p = f.path.trim();
      if (!n && !p) continue;
      if (!n || !p) {
        setError("In extra fields, fill in both the short name and the event field path.");
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
      setError("Cannot connect to API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    setSaving(true);
    setError(null);
    const err = await onDelete(initial.schema);
    if (err) setError(`Failed to delete mapping: ${err}`);
    setSaving(false);
  }

  return (
    <div className="mp-editor-clean">
      <header className="mp-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? (schema ? `New mapping: ${schema}` : "New mapping") : initial.schema}
        </Heading>
      </header>

      {sampleFields.length > 0 && (
        <Card title="Detected fields in sample event (click to use)">
          <div className="mp-sample-chips">
            {sampleFields.map((sf) => (
              <div key={sf.path} className="mp-chip-row">
                <code className="mp-chip-path">{sf.path}</code>
                <span className="mp-chip-val">{sf.preview}</span>
                <div className="mp-chip-btns">
                  <Button spacing="compact" onClick={() => setTitle(sf.path)}>
                    Use as title
                  </Button>
                  <Button spacing="compact" onClick={() => addGroupField(sf.path)}>
                    Use for grouping
                  </Button>
                  <Button spacing="compact" onClick={() => addExtraField(sf.path)}>
                    Add field
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {/* 1. Event type & alert title */}
      <Card title="1. Event schema & alert title">
        <div className="mp-two-col">
          <Field label="Event schema name">
            {(f) => (
              <TextInput
                {...f}
                value={schema}
                isDisabled={!isNew && Boolean(initial.schema)}
                onChange={(e) => setSchema(e.currentTarget.value)}
                placeholder="e.g. orders.failed.v1"
              />
            )}
          </Field>
          <Field
            label="Alert title field"
            hint="Field name from the event (e.g. error.message or annotations.summary)"
          >
            {(f) => (
              <TextInput
                {...f}
                value={title}
                onChange={(e) => setTitle(e.currentTarget.value)}
                placeholder="e.g. error.message"
              />
            )}
          </Field>
        </div>
      </Card>

      {/* 2. Alert severity */}
      <Card title="2. How should alert severity be determined?">
        <div className="mp-sev-mode">
          <Segmented
            label="Severity source"
            value={sevMode}
            onChange={(m) => setSevMode(m)}
            options={[
              { value: "const", label: "Always the same severity" },
              { value: "path", label: "Read from an event field" },
            ]}
          />
        </div>

        {sevMode === "const" ? (
          <Field label="Severity for all events of this schema">
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
            <Field label="Severity field in event" hint="e.g. severity or labels.severity">
              {(f) => (
                <TextInput
                  {...f}
                  value={sevPath}
                  onChange={(e) => setSevPath(e.currentTarget.value)}
                  placeholder="e.g. severity"
                />
              )}
            </Field>
            <Field label="Default severity (when field is missing)">
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

      {/* 3. Deduplication / grouping */}
      <Card title="3. Which fields group duplicate alerts?">
        <Text as="p" size="small" color="color.text.subtle">
          Events with matching values for these fields are merged into a single alert instead of creating duplicates.
        </Text>
        <div className="mp-field-list">
          {groupFields.map((g, idx) => (
            <div key={idx} className="mp-field-row">
              <div className="mp-grow">
                <Field label={idx === 0 ? "Grouping field" : "Additional field"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={g}
                      onChange={(e) => updateGroupField(idx, e.currentTarget.value)}
                      placeholder="e.g. labels.service or workflowName"
                    />
                  )}
                </Field>
              </div>
              <div className="mp-row-btn">
                <Button appearance="subtle" onClick={() => removeGroupField(idx)}>
                  Remove
                </Button>
              </div>
            </div>
          ))}
        </div>
        <div className="mp-add-btn">
          <Button onClick={() => addGroupField("")}>+ Add grouping field</Button>
        </div>
      </Card>

      {/* 4. Extra fields */}
      <Card title="4. Extra fields for alerts and rules (optional)">
        {extraFields.length === 0 ? (
          <Text as="p" size="small" color="color.text.subtle">
            No extra fields. Add fields you want to filter on in rules (e.g. service, desk, region).
          </Text>
        ) : (
          <div className="mp-field-list">
            {extraFields.map((ef, idx) => (
              <div key={ef.uid} className="mp-extra-row">
                <Field label={idx === 0 ? "Short name (e.g. service)" : "Short name"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={ef.name}
                      onChange={(e) => updateExtraField(ef.uid, { name: e.currentTarget.value })}
                      placeholder="e.g. service"
                    />
                  )}
                </Field>
                <Field label={idx === 0 ? "Path in event (e.g. labels.service)" : "Path in event"}>
                  {(f) => (
                    <TextInput
                      {...f}
                      value={ef.path}
                      onChange={(e) => updateExtraField(ef.uid, { path: e.currentTarget.value })}
                      placeholder="e.g. labels.service"
                    />
                  )}
                </Field>
                <div className="mp-row-btn">
                  <Button appearance="subtle" onClick={() => removeExtraField(ef.uid)}>
                    Remove
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
        <div className="mp-add-btn">
          <Button onClick={() => addExtraField("")}>+ Add field</Button>
        </div>
      </Card>

      {error && <ErrorBox message={error} />}

      <div className="mp-savebar">
        <Button appearance="primary" isLoading={saving} isDisabled={saving} onClick={() => void handleSave()}>
          {isNew ? "Create mapping" : "Save changes"}
        </Button>
        {!isNew && (
          <ConfirmInline
            label="Delete mapping"
            question={`Delete mapping "${initial.schema}"?`}
            confirmLabel="Yes, delete"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </div>
  );
}
