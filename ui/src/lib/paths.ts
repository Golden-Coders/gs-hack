
export type PathSeg = string | number;

const IDENT = /^[A-Za-z_][A-Za-z0-9_-]*$/;

/** Klucz porównawczy ścieżki (do oznaczania zaznaczonych pól w drzewie). */
export function pathKey(segs: PathSeg[]): string {
  return segs.join(".");
}

/** `["labels","service"]` -> `$.labels.service`; nietypowe klucze w nawiasach: `$['a b']`. */
export function toJsonPath(segs: PathSeg[]): string {
  let out = "$";
  for (const s of segs) {
    if (typeof s === "number") out += `[${s}]`;
    else if (IDENT.test(s)) out += `.${s}`;
    else out += `['${s.replace(/'/g, "\\'")}']`;
  }
  return out;
}

/** Ścieżka dla warunków (`raw.` + segmenty po kropce). */
export function toConditionField(segs: PathSeg[]): string {
  return ["raw", ...segs].join(".");
}

/** Parsuje prosty JSONPath (`$`, `.klucz`, `['klucz']`, `[n]`); dla składni zaawansowanych zwraca null. */
export function parseJsonPath(path: string): PathSeg[] | null {
  const p = path.trim();
  if (!p.startsWith("$")) return null;
  const segs: PathSeg[] = [];
  let i = 1;
  while (i < p.length) {
    const c = p[i];
    if (c === ".") {
      const m = /^[A-Za-z0-9_-]+/.exec(p.slice(i + 1));
      if (!m) return null;
      segs.push(m[0]);
      i += 1 + m[0].length;
    } else if (c === "[") {
      const end = p.indexOf("]", i);
      if (end < 0) return null;
      const inner = p.slice(i + 1, end);
      if (/^\d+$/.test(inner)) segs.push(Number(inner));
      else {
        const m = /^(['"])(.*)\1$/.exec(inner);
        if (!m) return null;
        segs.push(m[2].replace(/\\'/g, "'"));
      }
      i = end + 1;
    } else return null;
  }
  return segs;
}

/** Ścieżka warunku `raw.a.b` -> segmenty (liczby jako indeksy); inna pierwsza część niż `raw` -> null. */
export function parseConditionField(field: string): PathSeg[] | null {
  const parts = field.split(".");
  if (parts[0] !== "raw") return null;
  return parts.slice(1).map((s) => (/^\d+$/.test(s) ? Number(s) : s));
}

export function getAt(root: unknown, segs: PathSeg[]): unknown {
  let cur: unknown = root;
  for (const s of segs) {
    if (cur === null || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[String(s)];
  }
  return cur;
}

/** Domyślna nazwa pola w mappingu: ostatni klucz; dla indeksu – klucz nadrzędny + indeks. */
export function defaultFieldName(segs: PathSeg[]): string {
  const last = segs[segs.length - 1];
  if (typeof last === "string") return last;
  const parent = [...segs].reverse().find((s) => typeof s === "string");
  return `${parent ?? "pole"}${last ?? ""}`;
}
