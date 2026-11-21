import type { Schemas } from "../../api/client";

export type Mapping = Schemas["MappingDef"];
export type Cond = Schemas["Condition"];
export type Severity = Schemas["Severity"];

export const SEVERITIES: Severity[] = ["critical", "error", "warning", "info"];

/** Operatory, które edytor pokazuje w formularzu; warunki z innymi (np. `in`) są tylko do odczytu. */
export const SIMPLE_OPS = ["eq", "ne", "contains", "gt", "gte", "lt", "lte", "exists", "regex", "glob"] as const;
export type SimpleOp = (typeof SIMPLE_OPS)[number];

export const OP_LABEL: Record<SimpleOp, string> = {
  eq: "jest równe",
  ne: "jest różne od",
  contains: "zawiera",
  gt: "większe niż",
  gte: "większe lub równe",
  lt: "mniejsze niż",
  lte: "mniejsze lub równe",
  exists: "istnieje",
  regex: "pasuje do regex",
  glob: "pasuje do wzorca",
};

const NUMERIC_OPS: string[] = ["gt", "gte", "lt", "lte"];

/** Warunek-liść z operatorem obsługiwanym przez formularz (bez all/any/not). */
export function isSimpleLeaf(c: Cond | null | undefined): c is Cond & { field: string; op: SimpleOp } {
  return (
    !!c &&
    c.field !== undefined &&
    !c.all &&
    !c.any &&
    !c.not &&
    (SIMPLE_OPS as readonly string[]).includes(c.op ?? "eq")
  );
}

/** Wartość z pola tekstowego z zachowaniem typu: operatory liczbowe -> liczba, inaczej typ poprzedniej wartości. */
export function coerceValue(text: string, prev: unknown, op: string): unknown {
  const asNumber = text.trim() !== "" && Number.isFinite(Number(text));
  if (NUMERIC_OPS.includes(op)) return asNumber ? Number(text) : text;
  if (typeof prev === "number" && asNumber) return Number(text);
  if (typeof prev === "boolean" && (text === "true" || text === "false")) return text === "true";
  return text;
}

export function valueToText(v: unknown): string {
  if (v === undefined || v === null) return "";
  return typeof v === "string" ? v : JSON.stringify(v);
}

export type FieldRow = { id: number; name: string; path: string };
export type MapRow = { id: number; key: string; value: Severity };
export type RuleRow = { id: number; when: Cond; value: Severity };

export type SeverityDraft =
  | { type: "const"; value: Severity }
  | { type: "path"; path: string; map: MapRow[]; default: Severity }
  | { type: "rules"; rules: RuleRow[]; default: Severity };

export type Draft = {
  schema: string;
  version?: number;
  fields: FieldRow[];
  title: string;
  severity: SeverityDraft;
  fingerprint: string[];
  resolved: Cond | null;
};

let seq = 1;
export const nextId = () => seq++;

export function emptyDraft(schema: string): Draft {
  return {
    schema,
    fields: [],
    title: "",
    severity: { type: "const", value: "warning" },
    fingerprint: [],
    resolved: null,
  };
}

export function draftFromMapping(m: Mapping): Draft {
  const s = m.severity;
  let severity: SeverityDraft;
  if (s.type === "path") {
    severity = {
      type: "path",
      path: s.path ?? "",
      map: Object.entries(s.map ?? {}).map(([key, value]) => ({ id: nextId(), key, value })),
      default: s.default ?? "warning",
    };
  } else if (s.type === "rules") {
    severity = {
      type: "rules",
      rules: (s.rules ?? []).map((r) => ({ id: nextId(), when: r.when, value: r.value })),
      default: s.default ?? "warning",
    };
  } else severity = { type: "const", value: s.value ?? "warning" };
  return {
    schema: m.schema,
    version: m.version,
    fields: Object.entries(m.fields ?? {}).map(([name, path]) => ({ id: nextId(), name, path })),
    title: m.title ?? "",
    severity,
    fingerprint: [...(m.fingerprint ?? [])],
    resolved: m.resolved ?? null,
  };
}

export function mappingFromDraft(d: Draft): Mapping {
  const s = d.severity;
  const severity: Mapping["severity"] =
    s.type === "const"
      ? { type: "const", value: s.value }
      : s.type === "path"
        ? {
            type: "path",
            path: s.path.trim(),
            map: Object.fromEntries(s.map.filter((r) => r.key.trim() !== "").map((r) => [r.key, r.value])),
            default: s.default,
          }
        : { type: "rules", rules: s.rules.map((r) => ({ when: r.when, value: r.value })), default: s.default };
  const m: Mapping = {
    schema: d.schema.trim(),
    fields: Object.fromEntries(d.fields.filter((f) => f.name.trim() !== "").map((f) => [f.name.trim(), f.path.trim()])),
    title: d.title.trim() === "" ? null : d.title.trim(),
    severity,
    fingerprint: d.fingerprint.map((p) => p.trim()).filter(Boolean),
    resolved: d.resolved,
  };
  if (d.version !== undefined) m.version = d.version;
  return m;
}

export type Problems = {
  schema?: string;
  fields: Record<number, string>;
  title?: string;
  severity?: string;
  fingerprint: Record<number, string>;
  resolved?: string;
  any: boolean;
};

const JSONPATH = /^\$/;

/** Walidacja po stronie klienta: komunikaty stoją przy polach. Backend waliduje dokładniej (400). */
export function validate(d: Draft): Problems {
  const p: Problems = { fields: {}, fingerprint: {}, any: false };
  if (!d.schema.trim()) p.schema = "Podaj nazwę schematu.";
  const seen = new Set<string>();
  for (const f of d.fields) {
    const name = f.name.trim();
    if (!name) p.fields[f.id] = "Podaj nazwę pola.";
    else if (seen.has(name)) p.fields[f.id] = "Nazwa pola się powtarza.";
    else if (!JSONPATH.test(f.path.trim())) p.fields[f.id] = "Ścieżka zaczyna się od $ (np. $.labels.service).";
    seen.add(name);
  }
  if (d.title.trim() && !d.title.includes("{{") && !JSONPATH.test(d.title.trim())) {
    p.title = "Tytuł to ścieżka ($.pole) albo szablon z {{...}}.";
  }
  const s = d.severity;
  if (s.type === "path" && !JSONPATH.test(s.path.trim())) p.severity = "Wskaż pole severity w payloadzie (ścieżka zaczyna się od $).";
  if (s.type === "rules" && s.rules.length === 0) p.severity = "Dodaj co najmniej jedną regułę albo wybierz inny tryb.";
  d.fingerprint.forEach((path, i) => {
    if (!JSONPATH.test(path.trim())) p.fingerprint[i] = "Ścieżka zaczyna się od $.";
  });
  if (d.resolved && isSimpleLeaf(d.resolved)) {
    if (!d.resolved.field.trim()) p.resolved = "Wskaż pole payloadu.";
    else if (d.resolved.op !== "exists" && valueToText(d.resolved.value) === "") p.resolved = "Podaj wartość do porównania.";
  }
  p.any =
    !!p.schema ||
    !!p.title ||
    !!p.severity ||
    !!p.resolved ||
    Object.keys(p.fields).length > 0 ||
    Object.keys(p.fingerprint).length > 0;
  return p;
}
