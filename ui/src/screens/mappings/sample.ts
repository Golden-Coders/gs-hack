// Przykładowy payload z UNMAPPED: koperta ma `format` json albo yaml; dla YAML API zwraca tekst,
// więc w UI parsujemy go do drzewa.
import { parse } from "yaml";

export type ParsedSample = { value: unknown; format: "json" | "yaml" | "invalid"; error?: string };

/** Dla stringa próbuje JSON, potem YAML; obiekt/tablica z API (format json) przechodzi bez zmian. */
export function parseSample(sample: unknown): ParsedSample {
  if (typeof sample !== "string") return { value: sample, format: "json" };
  try {
    return { value: JSON.parse(sample), format: "json" };
  } catch {
    // nie JSON – próbujemy YAML
  }
  try {
    const value: unknown = parse(sample);
    if (value !== null && typeof value === "object") return { value, format: "yaml" };
    return { value: sample, format: "invalid", error: "Tekst nie jest obiektem JSON ani YAML." };
  } catch (e) {
    return { value: sample, format: "invalid", error: e instanceof Error ? e.message : String(e) };
  }
}
