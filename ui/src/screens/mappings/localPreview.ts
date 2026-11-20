// Przybliżony podgląd mappingu liczony w przeglądarce – zapas na wypadek, gdy `POST /mappings/preview`
// nie jest dostępny (backend zwraca 501). Źródłem prawdy jest backend (crate lifecycle); tu tylko prosta ścieżka
// i warunki-liście, więc wynik jest oznaczany w UI jako „lokalny”.
import type { Mapping, Cond, Severity } from "./draft";
import { getAt, parseConditionField, parseJsonPath } from "./paths";

export type PreviewResult = {
  severity: Severity;
  title: string;
  fingerprint: string;
  resolved: boolean;
  fields: Record<string, unknown>;
  origin: "server" | "local";
  mappingVersion?: number;
};

function query(raw: unknown, path: string): unknown {
  const segs = parseJsonPath(path);
  return segs ? getAt(raw, segs) : undefined;
}

function asText(v: unknown): string | null {
  if (v === undefined || v === null) return null;
  return typeof v === "string" ? v : JSON.stringify(v);
}

const SCALE: Record<string, Severity> = { critical: "critical", crit: "critical", error: "error", warning: "warning", warn: "warning", info: "info" };

function globToRegex(glob: string): RegExp {
  const esc = glob.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${esc}$`);
}

function leaf(c: Cond, raw: unknown): boolean {
  const segs = parseConditionField(c.field ?? "");
  const actual = segs ? getAt(raw, segs) : undefined;
  const op = c.op ?? "eq";
  const v = c.value;
  switch (op) {
    case "exists":
      return actual !== undefined && actual !== null;
    case "eq":
      return actual !== undefined && String(actual) === String(v);
    case "ne":
      return actual === undefined || String(actual) !== String(v);
    case "gt":
      return Number(actual) > Number(v);
    case "gte":
      return Number(actual) >= Number(v);
    case "lt":
      return Number(actual) < Number(v);
    case "lte":
      return Number(actual) <= Number(v);
    case "contains":
      return typeof actual === "string" ? actual.includes(String(v)) : Array.isArray(actual) && actual.map(String).includes(String(v));
    case "regex":
      try {
        return new RegExp(String(v)).test(String(actual ?? ""));
      } catch {
        return false;
      }
    case "glob":
      return globToRegex(String(v)).test(String(actual ?? ""));
    case "in":
      return Array.isArray(v) && actual !== undefined && v.map(String).includes(String(actual));
    default:
      return false;
  }
}

export function evalCondition(c: Cond, raw: unknown): boolean {
  const parts: boolean[] = [];
  if (c.all) parts.push(c.all.every((x) => evalCondition(x, raw)));
  if (c.any) parts.push(c.any.some((x) => evalCondition(x, raw)));
  if (c.not) parts.push(!evalCondition(c.not, raw));
  if (c.field !== undefined) parts.push(leaf(c, raw));
  return parts.every(Boolean);
}

export function localPreview(m: Mapping, raw: unknown): PreviewResult {
  const fields: Record<string, unknown> = {};
  for (const [name, path] of Object.entries(m.fields ?? {})) fields[name] = query(raw, path) ?? null;

  const t = (m.title ?? "").trim();
  let title = "";
  if (t.includes("{{")) {
    title = t.replace(/\{\{\s*([^}]+?)\s*\}\}/g, (_, expr: string) => {
      const segs = parseConditionField(expr.trim());
      return asText(segs ? getAt(raw, segs) : fields[expr.trim()]) ?? "";
    });
  } else if (t) title = asText(query(raw, t)) ?? "";
  if (!title.trim()) title = m.schema;

  const s = m.severity;
  let severity: Severity;
  if (s.type === "const") severity = s.value ?? "warning";
  else if (s.type === "path") {
    const v = asText(query(raw, s.path ?? ""));
    const map = s.map ?? {};
    const hit = v === null ? undefined : (map[v] ?? Object.entries(map).find(([k]) => k.toLowerCase() === v.toLowerCase())?.[1] ?? SCALE[v.toLowerCase()]);
    severity = hit ?? s.default ?? "warning";
  } else {
    severity = (s.rules ?? []).find((r) => evalCondition(r.when, raw))?.value ?? s.default ?? "warning";
  }

  const fp = (m.fingerprint ?? []).map((p) => asText(query(raw, p)) ?? "");
  const fingerprint = fp.length ? [m.schema, ...fp].join("|") : `${m.schema}|(skrót całego payloadu)`;

  return { severity, title, fingerprint, resolved: m.resolved ? evalCondition(m.resolved, raw) : false, fields, origin: "local" };
}
