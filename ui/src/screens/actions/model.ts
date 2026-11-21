import type { Schemas } from "../../api/client";

export type ActionDef = Schemas["ActionDef"];

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"] as const;
export const DEFAULT_TIMEOUT_MS = 5000;
/** Nazwy zajęte przez akcje wbudowane i przez ścieżkę `/actions/missing`. */
export const RESERVED_NAMES = ["escalate", "notify", "http_get", "missing"];

export type HeaderRow = { id: number; name: string; value: string };

export type FormState = {
  name: string;
  description: string;
  risk: "low" | "high";
  method: string;
  url: string;
  headers: HeaderRow[];
  body: string;
  timeout: string;
  paramsSchema: string;
};

export type FieldErrors = Partial<Record<"name" | "url" | "timeout" | "paramsSchema" | "headers", string>>;

let rowSeq = 0;
export const newRow = (name = "", value = ""): HeaderRow => ({ id: ++rowSeq, name, value });

export function emptyForm(name = ""): FormState {
  return {
    name,
    description: "",
    risk: "low",
    method: "POST",
    url: "",
    headers: [],
    body: "",
    timeout: String(DEFAULT_TIMEOUT_MS),
    paramsSchema: "",
  };
}

export function fromAction(a: ActionDef): FormState {
  return {
    name: a.name,
    description: a.description ?? "",
    risk: a.risk,
    method: a.http?.method ?? "POST",
    url: a.http?.url ?? "",
    headers: Object.entries(a.http?.headers ?? {}).map(([k, v]) => newRow(k, v)),
    body: a.http?.body ?? "",
    timeout: String(a.http?.timeout_ms ?? DEFAULT_TIMEOUT_MS),
    paramsSchema: a.params_schema == null ? "" : JSON.stringify(a.params_schema, null, 2),
  };
}

const PLAIN_HEADERS = /^(content-type|accept|accept-language|accept-encoding|user-agent|content-length|cache-control)$/i;
const SECRET_NAME = /authorization|api[-_]?key|token|secret|password|passwd|cookie|signature/i;
const TOKEN_LIKE = /^[A-Za-z0-9._~+/=-]{24,}$/;
const ENV_REF = /\$\{[A-Za-z_][A-Za-z0-9_]*\}/;

/**
 * Czy wartość nagłówka wygląda na wklejony sekret (a nie referencję `${ENV}`)?
 * Heurystyka: długi ciąg znaków tokenu albo nagłówek o „sekretnej” nazwie z wartością bez `${...}`.
 */
export function looksLikePastedSecret(name: string, value: string): boolean {
  const v = value.trim();
  if (!v || ENV_REF.test(v) || PLAIN_HEADERS.test(name.trim())) return false;
  const stripped = v.replace(/^(bearer|basic|token)\s+/i, "");
  if (TOKEN_LIKE.test(stripped)) return true;
  return SECRET_NAME.test(name) && stripped.length > 0 && !/^(bearer|basic|token)$/i.test(v);
}

export function validate(f: FormState, opts: { isNew: boolean; originalName?: string; takenNames: Set<string> }): FieldErrors {
  const e: FieldErrors = {};
  const name = f.name.trim();
  const isRenamed = !opts.isNew && opts.originalName !== undefined && name !== opts.originalName;
  if (!name) e.name = "Podaj nazwę akcji.";
  else if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(name))
    e.name = "Nazwa: litery, cyfry, „_” i „-”, zaczyna się od litery (np. flush_cache).";
  else if ((opts.isNew || isRenamed) && (RESERVED_NAMES.includes(name) || opts.takenNames.has(name)))
    e.name = "Akcja o tej nazwie już istnieje (albo nazwa jest zarezerwowana).";

  const url = f.url.trim();
  if (!url) e.url = "Podaj adres URL (szablon).";
  else if (!/^https?:\/\//i.test(url)) e.url = "URL musi zaczynać się od http:// lub https://.";

  if (!/^\d+$/.test(f.timeout.trim()) || Number(f.timeout) < 1) e.timeout = "Timeout: dodatnia liczba milisekund.";
  else if (Number(f.timeout) > 300_000) e.timeout = "Timeout może mieć najwyżej 300000 ms.";

  const seen = new Set<string>();
  for (const h of f.headers) {
    const k = h.name.trim().toLowerCase();
    if (!k && !h.value.trim()) continue;
    if (!k) e.headers = "Nagłówek bez nazwy.";
    else if (!/^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/.test(k)) e.headers = `Niepoprawna nazwa nagłówka: ${h.name}.`;
    else if (seen.has(k)) e.headers = `Nagłówek ${h.name} występuje więcej niż raz.`;
    seen.add(k);
  }

  if (f.paramsSchema.trim()) {
    try {
      const parsed: unknown = JSON.parse(f.paramsSchema);
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed))
        e.paramsSchema = "Schemat parametrów musi być obiektem JSON (JSON Schema).";
    } catch {
      e.paramsSchema = "Niepoprawny JSON.";
    }
  }
  return e;
}

/** Stan formularza -> ActionDef (zakłada, że `validate` przeszło). */
export function toAction(f: FormState, version?: number): ActionDef {
  const headers: Record<string, string> = {};
  for (const h of f.headers) if (h.name.trim()) headers[h.name.trim()] = h.value;
  return {
    name: f.name.trim(),
    kind: "http",
    risk: f.risk,
    description: f.description.trim() || null,
    params_schema: f.paramsSchema.trim() ? (JSON.parse(f.paramsSchema) as unknown) : null,
    http: {
      method: f.method,
      url: f.url.trim(),
      headers,
      body: f.body.trim() ? f.body : null,
      timeout_ms: Number(f.timeout),
    },
    ...(version !== undefined ? { version } : {}),
  };
}
