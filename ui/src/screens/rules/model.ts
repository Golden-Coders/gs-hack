// Model edytora reguł (tasks.md 15.1): drzewo warunków do edycji w kreatorze, formularz reguły,
// konwersje RuleDef <-> formularz <-> YAML oraz walidacja. Ewaluacją warunków zajmuje się backend (crate `condition`).
import { LineCounter, parseDocument, stringify } from "yaml";
import type { Schemas } from "../../api/client";
import { PALETTE_NAMES, type PaletteName } from "../../components/colors";

export type Rule = Schemas["RuleDef"];
export type Cond = Schemas["Condition"];
export type Op = NonNullable<Cond["op"]>;
export type Severity = Schemas["Severity"];
export type Recipient = Schemas["Recipient"];

// ---------------------------------------------------------------- słowniki

export const OPS: { op: Op; label: string; hint?: string }[] = [
  { op: "eq", label: "równa się" },
  { op: "ne", label: "jest różne od" },
  { op: "in", label: "jest jedną z wartości", hint: "wartości oddzielone przecinkami, np. TIMEOUT, REJECTED" },
  { op: "gt", label: "jest większe niż", hint: "liczba" },
  { op: "gte", label: "jest większe lub równe", hint: "liczba" },
  { op: "lt", label: "jest mniejsze niż", hint: "liczba" },
  { op: "lte", label: "jest mniejsze lub równe", hint: "liczba" },
  { op: "contains", label: "zawiera" },
  { op: "regex", label: "pasuje do wyrażenia regularnego", hint: "np. ^payments-.*" },
  { op: "exists", label: "istnieje" },
  { op: "glob", label: "pasuje do wzorca", hint: "* zastępuje dowolny tekst, np. trade.rejected.*" },
];

export const OP_LABEL: Record<string, string> = Object.fromEntries(OPS.map((o) => [o.op, o.label]));
const OP_SET = new Set<string>(OPS.map((o) => o.op));
const NUMERIC_OPS = new Set<Op>(["gt", "gte", "lt", "lte"]);

export const SEVERITIES: { value: Severity; label: string }[] = [
  { value: "info", label: "info" },
  { value: "warning", label: "warning" },
  { value: "error", label: "error" },
  { value: "critical", label: "critical" },
];

// Nazwy kolorów pochodzą z palety (components/colors.ts); tu tylko polskie etykiety do selecta.
const COLOR_LABELS: Record<PaletteName, string> = {
  red: "czerwony",
  orange: "pomarańczowy",
  yellow: "żółty",
  green: "zielony",
  blue: "niebieski",
  purple: "fioletowy",
  gray: "szary",
};
export const COLORS: { value: PaletteName; label: string }[] = PALETTE_NAMES.map((value) => ({ value, label: COLOR_LABELS[value] }));
const COLOR_SET: ReadonlySet<string> = new Set(PALETTE_NAMES);

/** Pola kanoniczne zdarzenia – podpowiadane zawsze, niezależnie od `GET /fields`. */
export const CANONICAL_FIELDS = ["schema", "severity", "source", "title"];

// ---------------------------------------------------------------- drzewo warunków (model kreatora)

export interface LeafNode {
  uid: string;
  kind: "leaf";
  field: string;
  op: Op;
  /** Wartość jako tekst w polu formularza. */
  value: string;
  /** Oryginalna wartość z konfiguracji – zachowujemy jej typ, dopóki użytkownik jej nie zmieni. */
  raw?: unknown;
}
export interface GroupNode {
  uid: string;
  kind: "all" | "any";
  children: TreeNode[];
}
export interface NotNode {
  uid: string;
  kind: "not";
  child: TreeNode | null;
}
export type TreeNode = LeafNode | GroupNode | NotNode;

let seq = 0;
export const newUid = () => `n${++seq}`;

export const newLeaf = (): LeafNode => ({ uid: newUid(), kind: "leaf", field: "", op: "eq", value: "" });
export const newGroup = (kind: "all" | "any" = "all"): GroupNode => ({ uid: newUid(), kind, children: [] });
export const newNot = (): NotNode => ({ uid: newUid(), kind: "not", child: newLeaf() });

export function valueToText(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (Array.isArray(v)) return v.map((x) => valueToText(x)).join(", ");
  return JSON.stringify(v);
}

function scalar(text: string): unknown {
  const t = text.trim();
  if (t === "true") return true;
  if (t === "false") return false;
  if (t !== "" && Number.isFinite(Number(t)) && String(Number(t)) === t) return Number(t);
  return text;
}

/** Tekst z pola formularza -> wartość w regule (liczby dla porównań, lista dla `in`, tekst dla reszty). */
export function textToValue(op: Op, text: string, raw?: unknown): unknown {
  if (raw !== undefined && valueToText(raw) === text) return raw;
  if (op === "exists") return undefined;
  if (op === "in") {
    return text
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s !== "")
      .map(scalar);
  }
  if (NUMERIC_OPS.has(op)) {
    const t = text.trim();
    return t !== "" && Number.isFinite(Number(t)) ? Number(t) : text;
  }
  if (op === "eq" || op === "ne") return scalar(text);
  return text;
}

function conditionParts(c: Cond): TreeNode[] {
  const out: TreeNode[] = [];
  if (c.all) out.push({ uid: newUid(), kind: "all", children: c.all.map(conditionToNode) });
  if (c.any) out.push({ uid: newUid(), kind: "any", children: c.any.map(conditionToNode) });
  if (c.not) out.push({ uid: newUid(), kind: "not", child: conditionToNode(c.not) });
  if (c.field !== undefined) {
    const op = (c.op ?? "eq") as Op;
    out.push({ uid: newUid(), kind: "leaf", field: c.field, op, value: valueToText(c.value), raw: c.value });
  }
  return out;
}

export function conditionToNode(c: Cond): TreeNode {
  const parts = conditionParts(c);
  // Kilka kluczy w jednym obiekcie to koniunkcja – w kreatorze pokazujemy ją jawnie jako grupę „wszystkie”.
  return parts.length === 1 ? parts[0] : { uid: newUid(), kind: "all", children: parts };
}

/** Korzeń kreatora jest zawsze grupą (all/any). */
export function conditionToRoot(c: Cond | null | undefined): GroupNode {
  if (!c) return newGroup("all");
  const n = conditionToNode(c);
  return n.kind === "all" || n.kind === "any" ? n : { uid: newUid(), kind: "all", children: [n] };
}

export function nodeToCondition(n: TreeNode): Cond {
  if (n.kind === "leaf") {
    const c: Cond = { field: n.field, op: n.op };
    const v = textToValue(n.op, n.value, n.raw);
    if (v !== undefined) c.value = v;
    return c;
  }
  if (n.kind === "not") return { not: n.child ? nodeToCondition(n.child) : { all: [] } };
  return n.kind === "all" ? { all: n.children.map(nodeToCondition) } : { any: n.children.map(nodeToCondition) };
}

/** Zwraca nowe drzewo, w którym węzeł `uid` zastąpiono wynikiem `fn` (null = usunięcie). */
export function mapTree(node: TreeNode, uid: string, fn: (n: TreeNode) => TreeNode | null): TreeNode | null {
  if (node.uid === uid) return fn(node);
  if (node.kind === "leaf") return node;
  if (node.kind === "not") {
    const child = node.child ? mapTree(node.child, uid, fn) : null;
    return child === node.child ? node : { ...node, child };
  }
  const children = node.children.flatMap((c) => {
    const r = mapTree(c, uid, fn);
    return r ? [r] : [];
  });
  const same = children.length === node.children.length && children.every((c, i) => c === node.children[i]);
  return same ? node : { ...node, children };
}

export function findLeaves(node: TreeNode | null, out: LeafNode[] = []): LeafNode[] {
  if (!node) return out;
  if (node.kind === "leaf") out.push(node);
  else if (node.kind === "not") findLeaves(node.child, out);
  else node.children.forEach((c) => findLeaves(c, out));
  return out;
}

// ---------------------------------------------------------------- formularz reguły

export interface FieldRow {
  uid: string;
  path: string;
  /** Kolor wyróżnienia ("" = bez wyróżnienia). */
  color: string;
}

export interface RuleForm {
  id: string;
  name: string;
  team: string;
  enabled: boolean;
  severityOverride: "" | Severity;
  version: number;
  root: GroupNode;
  recipients: Recipient[];
  title: string;
  body: string;
  rows: FieldRow[];
  color: string;
}

export function emptyForm(): RuleForm {
  return {
    id: "",
    name: "",
    team: "",
    enabled: true,
    severityOverride: "",
    version: 0,
    root: newGroup("all"),
    recipients: [],
    title: "",
    body: "",
    rows: [],
    color: "",
  };
}

export function ruleToForm(rule: Rule): RuleForm {
  const p = rule.presentation ?? {};
  const highlight = p.highlight ?? {};
  const fields = p.fields ?? [];
  const rows: FieldRow[] = fields.map((path) => ({ uid: newUid(), path, color: highlight[path] ?? "" }));
  // Wyróżnione pola spoza listy `fields` pokazujemy jako wiersze (w kreatorze wyróżnienie dotyczy wyświetlanego pola).
  for (const [path, color] of Object.entries(highlight)) {
    if (!fields.includes(path)) rows.push({ uid: newUid(), path, color });
  }
  return {
    id: rule.id,
    name: rule.name,
    team: rule.team ?? "",
    enabled: rule.enabled ?? true,
    severityOverride: rule.severity_override ?? "",
    version: rule.version ?? 0,
    root: conditionToRoot(rule.condition),
    recipients: rule.recipients ?? [],
    title: p.title ?? "",
    body: p.body ?? "",
    rows,
    color: p.color ?? "",
  };
}

export function formToRule(f: RuleForm): Rule {
  const rows = f.rows.filter((r) => r.path.trim() !== "");
  return {
    id: f.id,
    name: f.name,
    team: f.team || null,
    condition: nodeToCondition(f.root),
    recipients: f.recipients,
    // Brak pola = brak nadpisania (serde `default`); `null` odrzuca walidacja mocka Prism (nullable w allOf).
    ...(f.severityOverride ? { severity_override: f.severityOverride } : {}),
    presentation: {
      title: f.title || null,
      body: f.body || null,
      fields: rows.map((r) => r.path.trim()),
      highlight: Object.fromEntries(rows.filter((r) => r.color).map((r) => [r.path.trim(), r.color])),
      color: f.color || null,
    },
    enabled: f.enabled,
    version: f.version,
  };
}

export function slugify(name: string): string {
  const map: Record<string, string> = { ą: "a", ć: "c", ę: "e", ł: "l", ń: "n", ó: "o", ś: "s", ź: "z", ż: "z" };
  return name
    .toLowerCase()
    .replace(/[ąćęłńóśźż]/g, (c) => map[c] ?? c)
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

// ---------------------------------------------------------------- walidacja kreatora

export interface LeafError {
  /** `required` – brak wartości (pokazujemy po próbie zapisu), `format` – zła postać (pokazujemy od razu). */
  kind: "required" | "format";
  field?: string;
  value?: string;
}

export interface FormErrors {
  id?: string;
  name?: string;
  recipients?: string;
  title?: string;
  body?: string;
  leaves: Record<string, LeafError>;
  /** Błędy grup (np. pusta negacja) po uid. */
  groups: Record<string, string>;
  /** Uwagi niewymuszające poprawki (np. pusta grupa). */
  warnings: Record<string, string>;
  count: number;
}

function templateError(t: string): string | undefined {
  const open = (t.match(/\{\{/g) ?? []).length;
  const close = (t.match(/\}\}/g) ?? []).length;
  return open !== close ? "Niedomknięte {{ }} w szablonie" : undefined;
}

export function validateForm(f: RuleForm, isNew: boolean): FormErrors {
  const e: FormErrors = { leaves: {}, groups: {}, warnings: {}, count: 0 };
  if (!f.name.trim()) e.name = "Podaj nazwę reguły";
  if (!f.id.trim()) e.id = "Podaj identyfikator";
  else if (isNew && !/^[a-z0-9][a-z0-9_-]*$/.test(f.id)) e.id = "Tylko małe litery, cyfry, myślnik i podkreślenie";
  if (f.recipients.length === 0) e.recipients = "Wybierz co najmniej jednego odbiorcę";
  const te = templateError(f.title);
  if (te) e.title = te;
  const be = templateError(f.body);
  if (be) e.body = be;

  const walk = (n: TreeNode) => {
    if (n.kind === "leaf") {
      const le: LeafError = { kind: "required" };
      let bad = false;
      if (!n.field.trim()) {
        le.field = "Wybierz pole";
        bad = true;
      }
      if (n.op !== "exists") {
        const text = n.value;
        if (text.trim() === "") {
          le.value = "Podaj wartość";
          bad = true;
        } else if (NUMERIC_OPS.has(n.op) && !Number.isFinite(Number(text.trim()))) {
          le.value = "Wpisz liczbę";
          le.kind = "format";
          bad = true;
        } else if (n.op === "regex") {
          try {
            new RegExp(text);
          } catch {
            le.value = "Niepoprawne wyrażenie regularne";
            le.kind = "format";
            bad = true;
          }
        } else if (n.op === "in" && (textToValue("in", text) as unknown[]).length === 0) {
          le.value = "Podaj co najmniej jedną wartość";
          bad = true;
        }
      }
      if (bad) e.leaves[n.uid] = le;
    } else if (n.kind === "not") {
      if (!n.child) e.groups[n.uid] = "Negacja musi zawierać warunek";
      else walk(n.child);
    } else {
      if (n.children.length === 0) {
        e.warnings[n.uid] =
          n.kind === "all" ? "Pusta grupa jest zawsze spełniona" : "Pusta grupa nigdy nie jest spełniona";
      }
      n.children.forEach(walk);
    }
  };
  walk(f.root);
  e.count =
    Number(!!e.name) +
    Number(!!e.id) +
    Number(!!e.recipients) +
    Number(!!e.title) +
    Number(!!e.body) +
    Object.keys(e.leaves).length +
    Object.keys(e.groups).length;
  return e;
}

// ---------------------------------------------------------------- YAML

/** Reguła w YAML: tylko to, co ma sens dla człowieka (pola puste pomijamy, wersję pilnuje serwer). */
export function ruleToYaml(rule: Rule): string {
  const obj: Record<string, unknown> = { id: rule.id, name: rule.name };
  if (rule.team) obj.team = rule.team;
  obj.enabled = rule.enabled ?? true;
  obj.condition = rule.condition;
  obj.recipients = rule.recipients;
  if (rule.severity_override) obj.severity_override = rule.severity_override;
  const p = rule.presentation ?? {};
  const pres: Record<string, unknown> = {};
  if (p.title) pres.title = p.title;
  if (p.body) pres.body = p.body;
  if (p.fields?.length) pres.fields = p.fields;
  if (p.highlight && Object.keys(p.highlight).length) pres.highlight = p.highlight;
  if (p.color) pres.color = p.color;
  if (Object.keys(pres).length) obj.presentation = pres;
  return stringify(obj, { lineWidth: 0, indent: 2 });
}

type Path = (string | number)[];
class RuleShapeError extends Error {
  constructor(
    public path: Path,
    message: string,
  ) {
    super(message);
  }
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

function buildCondition(v: unknown, path: Path, allowEmpty = false): Cond {
  if (!isObj(v)) throw new RuleShapeError(path, "warunek musi być obiektem (all, any, not albo field/op/value)");
  const known = new Set(["all", "any", "not", "field", "op", "value"]);
  for (const k of Object.keys(v)) {
    if (!known.has(k)) throw new RuleShapeError([...path, k], `nieznany klucz „${k}” w warunku`);
  }
  const c: Cond = {};
  if (v.all !== undefined) {
    if (!Array.isArray(v.all)) throw new RuleShapeError([...path, "all"], "„all” musi być listą warunków");
    c.all = v.all.map((x, i) => buildCondition(x, [...path, "all", i]));
  }
  if (v.any !== undefined) {
    if (!Array.isArray(v.any)) throw new RuleShapeError([...path, "any"], "„any” musi być listą warunków");
    c.any = v.any.map((x, i) => buildCondition(x, [...path, "any", i]));
  }
  if (v.not !== undefined) c.not = buildCondition(v.not, [...path, "not"]);
  if (v.field !== undefined || v.op !== undefined) {
    if (typeof v.field !== "string" || v.field === "") {
      throw new RuleShapeError([...path, "field"], "brak nazwy pola („field”)");
    }
    const op = v.op === undefined ? "eq" : v.op;
    if (typeof op !== "string" || !OP_SET.has(op)) {
      throw new RuleShapeError([...path, "op"], `nieznany operator „${String(op)}” (dozwolone: ${[...OP_SET].join(", ")})`);
    }
    if (op !== "exists" && v.value === undefined) {
      throw new RuleShapeError([...path, "value"], `operator „${op}” wymaga wartości („value”)`);
    }
    c.field = v.field;
    c.op = op as Op;
    if (v.value !== undefined) c.value = v.value;
  } else if (v.value !== undefined) {
    throw new RuleShapeError([...path, "value"], "„value” bez „field”");
  }
  if (Object.keys(c).length === 0 && !allowEmpty) throw new RuleShapeError(path, "pusty warunek");
  return c;
}

function buildRecipient(v: unknown, path: Path): Recipient {
  if (typeof v === "string") {
    const m = /^(person|team):(.+)$/.exec(v.trim());
    if (m) return { type: m[1] as Recipient["type"], id: m[2].trim() };
    throw new RuleShapeError(path, "odbiorca jako tekst musi mieć postać person:<id> albo team:<id>");
  }
  if (!isObj(v)) throw new RuleShapeError(path, "odbiorca musi mieć postać {type: person|team, id: ...}");
  if (v.type !== "person" && v.type !== "team") {
    throw new RuleShapeError([...path, "type"], "„type” odbiorcy to person albo team");
  }
  if (typeof v.id !== "string" || v.id === "") throw new RuleShapeError([...path, "id"], "brak „id” odbiorcy");
  return { type: v.type, id: v.id };
}

function optString(v: unknown, path: Path, what: string): string | null {
  if (v === undefined || v === null) return null;
  if (typeof v !== "string") throw new RuleShapeError(path, `${what} musi być tekstem`);
  return v;
}

function buildColor(v: unknown, path: Path): string | null {
  const s = optString(v, path, "kolor");
  if (s !== null && !COLOR_SET.has(s)) {
    throw new RuleShapeError(path, `nieznany kolor „${s}” (dozwolone: ${[...COLOR_SET].join(", ")})`);
  }
  return s;
}

function buildRule(data: unknown, prevVersion: number): Rule {
  if (!isObj(data)) throw new RuleShapeError([], "reguła musi być obiektem (id, name, condition, recipients, …)");
  const known = new Set(["id", "name", "team", "condition", "recipients", "severity_override", "presentation", "enabled", "version"]);
  for (const k of Object.keys(data)) if (!known.has(k)) throw new RuleShapeError([k], `nieznany klucz „${k}”`);
  if (typeof data.id !== "string" || data.id === "") throw new RuleShapeError(["id"], "brak „id” reguły");
  if (typeof data.name !== "string" || data.name === "") throw new RuleShapeError(["name"], "brak „name” reguły");
  if (data.condition === undefined) throw new RuleShapeError(["condition"], "brak „condition”");
  const condition = buildCondition(data.condition, ["condition"], true);
  if (!Array.isArray(data.recipients)) throw new RuleShapeError(["recipients"], "„recipients” musi być listą odbiorców");
  const recipients = data.recipients.map((r, i) => buildRecipient(r, ["recipients", i]));
  let severity: Severity | null = null;
  if (data.severity_override !== undefined && data.severity_override !== null) {
    const s = data.severity_override;
    if (typeof s !== "string" || !SEVERITIES.some((x) => x.value === s)) {
      throw new RuleShapeError(["severity_override"], `nieznane severity (dozwolone: ${SEVERITIES.map((x) => x.value).join(", ")})`);
    }
    severity = s as Severity;
  }
  const pv = data.presentation ?? {};
  if (!isObj(pv)) throw new RuleShapeError(["presentation"], "„presentation” musi być obiektem");
  const pk = new Set(["title", "body", "fields", "highlight", "color"]);
  for (const k of Object.keys(pv)) {
    if (!pk.has(k)) throw new RuleShapeError(["presentation", k], `nieznany klucz „${k}” w prezentacji`);
  }
  let fields: string[] = [];
  if (pv.fields !== undefined && pv.fields !== null) {
    if (!Array.isArray(pv.fields) || pv.fields.some((f) => typeof f !== "string")) {
      throw new RuleShapeError(["presentation", "fields"], "„fields” musi być listą ścieżek pól (tekstów)");
    }
    fields = pv.fields as string[];
  }
  const highlight: Record<string, string> = {};
  if (pv.highlight !== undefined && pv.highlight !== null) {
    if (!isObj(pv.highlight)) throw new RuleShapeError(["presentation", "highlight"], "„highlight” musi mapować pole na kolor");
    for (const [k, c] of Object.entries(pv.highlight)) highlight[k] = buildColor(c, ["presentation", "highlight", k]) ?? "";
  }
  if (data.enabled !== undefined && typeof data.enabled !== "boolean") {
    throw new RuleShapeError(["enabled"], "„enabled” musi być true albo false");
  }
  return {
    id: data.id,
    name: data.name,
    team: optString(data.team, ["team"], "team"),
    condition,
    recipients,
    severity_override: severity,
    presentation: {
      title: optString(pv.title, ["presentation", "title"], "title"),
      body: optString(pv.body, ["presentation", "body"], "body"),
      fields,
      highlight,
      color: buildColor(pv.color, ["presentation", "color"]),
    },
    enabled: (data.enabled as boolean | undefined) ?? true,
    version: typeof data.version === "number" ? data.version : prevVersion,
  };
}

export type YamlResult = { ok: true; rule: Rule } | { ok: false; message: string; line?: number; col?: number };

/** Parsuje YAML reguły; błąd niesie komunikat i (jeśli się da) miejsce w tekście. */
export function parseRuleYaml(text: string, prevVersion: number): YamlResult {
  if (text.trim() === "") return { ok: false, message: "YAML jest pusty" };
  const lc = new LineCounter();
  const doc = parseDocument(text, { lineCounter: lc });
  if (doc.errors.length) {
    const e = doc.errors[0];
    const pos = e.linePos?.[0];
    return {
      ok: false,
      message: e.message.split("\n")[0].replace(/ at line \d+, column \d+:?\s*$/, ""),
      line: pos?.line,
      col: pos?.col,
    };
  }
  try {
    return { ok: true, rule: buildRule(doc.toJS(), prevVersion) };
  } catch (err) {
    if (!(err instanceof RuleShapeError)) return { ok: false, message: String(err) };
    // Wskaż najgłębszy istniejący węzeł na ścieżce błędu (brakujący klucz -> jego rodzic).
    for (let i = err.path.length; i >= 0; i--) {
      const node = doc.getIn(err.path.slice(0, i), true) as { range?: [number, number, number] } | undefined;
      if (node?.range) {
        const p = lc.linePos(node.range[0]);
        return { ok: false, message: err.message, line: p.line, col: p.col };
      }
    }
    return { ok: false, message: err.message };
  }
}

// ---------------------------------------------------------------- pomocnicze

/** Schemat do podpowiedzi pól: z liścia `schema = <wartość>` (dla `glob` z wildcardem nie zgadujemy). */
export function guessSchema(root: GroupNode): string {
  for (const l of findLeaves(root)) {
    if (l.field === "schema" && l.op === "eq" && l.value.trim()) return l.value.trim();
  }
  return "";
}

/** Szkielet zdarzenia z listy ścieżek pól (`raw.payload.desk` -> { raw: { payload: { desk: "" } } }). */
export function skeletonFromFields(paths: string[]): Record<string, unknown> {
  const root: Record<string, unknown> = {};
  for (const p of paths) {
    const parts = p.split(".");
    let cur = root;
    parts.forEach((part, i) => {
      if (i === parts.length - 1) cur[part] = cur[part] ?? "";
      else {
        const next = cur[part];
        cur[part] = isObj(next) ? next : {};
        cur = cur[part] as Record<string, unknown>;
      }
    });
  }
  return root;
}
