// Gwarancja D7: żadnych surowych kolorów w kodzie UI — tylko tokeny ADS (`token()` / `var(--ds-*)`).
// ESLint (`ensure-design-token-usage`) nie widzi plików .css, więc sprawdzamy je tutaj.
// Pomijamy wygenerowany kontrakt i testy (mogą podawać hex jako dane wejściowe, np. colors.test.ts).
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..");
const RAW = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/;

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory()) return files(p);
    return /\.(css|ts|tsx)$/.test(e.name) && !/\.test\.ts$/.test(e.name) && !p.endsWith(join("api", "schema.d.ts")) ? [p] : [];
  });
}

describe("brak surowych kolorów w ui/src", () => {
  it("pliki .css/.ts/.tsx używają tylko tokenów", () => {
    const hits = files(SRC).flatMap((f) =>
      readFileSync(f, "utf8")
        .split("\n")
        .flatMap((line, i) => (RAW.test(line) ? [`${relative(SRC, f)}:${i + 1}: ${line.trim()}`] : [])),
    );
    expect(hits).toEqual([]);
  });
});
