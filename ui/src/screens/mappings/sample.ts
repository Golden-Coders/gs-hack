// Przykładowy payload z UNMAPPED: koperta ma `format` json albo yaml; dla YAML API zwraca tekst,
// więc w UI parsujemy go do drzewa (minimalny parser podzbioru YAML – bez zależności).
// Obsługa: mapy i listy blokowe, skalary, cudzysłowy, komentarze, `|`/`>`; kotwice i tagi nie są wspierane.

export type ParsedSample = { value: unknown; format: "json" | "yaml" | "invalid"; error?: string };

type Line = { indent: number; text: string };

function stripComment(s: string): string {
  let q: string | null = null;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (q) {
      if (c === q) q = null;
    } else if (c === '"' || c === "'") q = c;
    else if (c === "#" && (i === 0 || /\s/.test(s[i - 1]))) return s.slice(0, i);
  }
  return s;
}

function scalar(raw: string): unknown {
  const s = raw.trim();
  if (s === "" || s === "~" || s === "null") return null;
  if (s === "true") return true;
  if (s === "false") return false;
  if (/^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(s)) return Number(s);
  if (s.startsWith('"') && s.endsWith('"') && s.length >= 2) {
    try {
      return JSON.parse(s);
    } catch {
      return s.slice(1, -1);
    }
  }
  if (s.startsWith("'") && s.endsWith("'") && s.length >= 2) return s.slice(1, -1).replace(/''/g, "'");
  if (s.startsWith("{") || s.startsWith("[")) {
    try {
      return JSON.parse(s);
    } catch {
      // flow-style spoza JSON (np. klucze bez cudzysłowów) zostaje tekstem
    }
  }
  return s;
}

function splitKey(text: string): [string, string] | null {
  const m = /^("[^"]*"|'[^']*'|[^\s:#\-[{][^:]*?|-[^\s:][^:]*?)\s*:(?:\s+(.*))?$/.exec(text);
  if (!m) return null;
  return [m[1].replace(/^(["'])(.*)\1$/, "$2"), m[2] ?? ""];
}

const isItem = (t: string) => t === "-" || t.startsWith("- ");

function parseBlock(lines: Line[], pos: { i: number }, indent: number): unknown {
  if (isItem(lines[pos.i].text)) {
    const arr: unknown[] = [];
    while (pos.i < lines.length && lines[pos.i].indent === indent && isItem(lines[pos.i].text)) {
      const rest = lines[pos.i].text.slice(1).trim();
      if (rest === "") {
        pos.i++;
        arr.push(pos.i < lines.length && lines[pos.i].indent > indent ? parseBlock(lines, pos, lines[pos.i].indent) : null);
      } else if (splitKey(rest)) {
        // „- klucz: wartość” – element jest mapą, jej kolejne klucze leżą wcięte o 2
        lines[pos.i] = { indent: indent + 2, text: rest };
        arr.push(parseBlock(lines, pos, indent + 2));
      } else {
        arr.push(scalar(rest));
        pos.i++;
      }
    }
    return arr;
  }
  const obj: Record<string, unknown> = {};
  while (pos.i < lines.length && lines[pos.i].indent === indent && !isItem(lines[pos.i].text)) {
    const kv = splitKey(lines[pos.i].text);
    if (!kv) throw new Error(`Nie rozumiem linii: „${lines[pos.i].text}”`);
    const [key, rest] = kv;
    pos.i++;
    const next = lines[pos.i];
    if (rest === "|" || rest === ">") {
      const parts: string[] = [];
      while (pos.i < lines.length && lines[pos.i].indent > indent) parts.push(lines[pos.i++].text);
      obj[key] = parts.join(rest === "|" ? "\n" : " ");
    } else if (rest !== "") {
      obj[key] = scalar(rest);
    } else if (next && (next.indent > indent || (next.indent === indent && isItem(next.text)))) {
      obj[key] = parseBlock(lines, pos, next.indent);
    } else {
      obj[key] = null;
    }
  }
  return obj;
}

export function parseYaml(text: string): unknown {
  const lines: Line[] = text
    .split(/\r?\n/)
    .map((l) => ({ indent: l.length - l.trimStart().length, text: stripComment(l).trim() }))
    .filter((l) => l.text !== "" && l.text !== "---");
  if (!lines.length) return null;
  if (!splitKey(lines[0].text) && !isItem(lines[0].text)) return scalar(lines.map((l) => l.text).join(" "));
  return parseBlock(lines, { i: 0 }, lines[0].indent);
}

/** Dla stringa próbuje JSON, potem YAML; obiekt/tablica z API (format json) przechodzi bez zmian. */
export function parseSample(sample: unknown): ParsedSample {
  if (typeof sample !== "string") return { value: sample, format: "json" };
  try {
    return { value: JSON.parse(sample), format: "json" };
  } catch {
    // nie JSON – próbujemy YAML
  }
  try {
    const value = parseYaml(sample);
    if (value !== null && typeof value === "object") return { value, format: "yaml" };
    return { value: sample, format: "invalid", error: "Tekst nie jest obiektem JSON ani YAML." };
  } catch (e) {
    return { value: sample, format: "invalid", error: e instanceof Error ? e.message : String(e) };
  }
}
