import { describe, expect, it } from "vitest";
import flushCache from "../../../../crates/contracts/fixtures/config/action.flush_cache.json";
import {
  DEFAULT_TIMEOUT_MS,
  emptyForm,
  fromAction,
  looksLikePastedSecret,
  newRow,
  toAction,
  validate,
  type ActionDef,
  type FormState,
} from "./model";

const flush = flushCache as unknown as ActionDef;
const opts = { isNew: true, takenNames: new Set<string>() };

function validForm(patch: Partial<FormState> = {}): FormState {
  return { ...emptyForm("restart_service"), url: "http://mock-targets:8090/restart", ...patch };
}

describe("fromAction / toAction", () => {
  it("round-trip fixture flush_cache zwraca ten sam ActionDef", () => {
    expect(toAction(fromAction(flush), flush.version)).toEqual(flush);
  });

  it("formularz z fixture ma poprawne pola i przechodzi walidację", () => {
    const f = fromAction(flush);
    expect(f).toMatchObject({ name: "flush_cache", risk: "high", method: "POST", timeout: "5000", paramsSchema: "" });
    expect(f.headers.map((h) => [h.name, h.value])).toEqual([["Authorization", "Bearer ${CACHE_ADMIN_TOKEN}"]]);
    expect(validate(f, { isNew: false, takenNames: new Set() })).toEqual({});
  });

  it("params_schema jest serializowany do JSON i z powrotem", () => {
    const schema = { type: "object", properties: { service: { type: "string" } }, required: ["service"] };
    const f = fromAction({ ...flush, params_schema: schema });
    expect(JSON.parse(f.paramsSchema)).toEqual(schema);
    expect(toAction(f).params_schema).toEqual(schema);
  });

  it("puste opcjonalne pola wracają jako null; nagłówki bez nazwy są pomijane", () => {
    const a = toAction(validForm({ description: "  ", body: "   ", paramsSchema: "", headers: [newRow("", "x"), newRow("X-A", "1")] }));
    expect(a.description).toBeNull();
    expect(a.params_schema).toBeNull();
    expect(a.http?.body).toBeNull();
    expect(a.http?.headers).toEqual({ "X-A": "1" });
    expect(a.kind).toBe("http");
  });

  it("wersja jest dołączana tylko gdy podana; timeout to liczba", () => {
    expect(toAction(validForm())).not.toHaveProperty("version");
    expect(toAction(validForm(), 4).version).toBe(4);
    expect(toAction(validForm({ timeout: "1500" })).http?.timeout_ms).toBe(1500);
  });

  it("brak http w akcji -> domyślne wartości formularza", () => {
    const f = fromAction({ name: "escalate", kind: "builtin", risk: "low" } as unknown as ActionDef);
    expect(f.method).toBe("POST");
    expect(f.url).toBe("");
    expect(f.timeout).toBe(String(DEFAULT_TIMEOUT_MS));
  });
});

describe("validate", () => {
  it("poprawny formularz nie ma błędów", () => {
    expect(validate(validForm(), opts)).toEqual({});
  });

  it("pusty URL", () => {
    expect(validate(validForm({ url: "  " }), opts).url).toBeTruthy();
  });

  it("URL musi mieć schemat http(s); szablon w URL jest dozwolony", () => {
    expect(validate(validForm({ url: "ftp://x" }), opts).url).toBeTruthy();
    expect(validate(validForm({ url: "mock-targets:8090/x" }), opts).url).toBeTruthy();
    expect(validate(validForm({ url: "HTTPS://x/{{service}}/flush" }), opts).url).toBeUndefined();
  });

  it("zły JSON w params_schema", () => {
    expect(validate(validForm({ paramsSchema: "{ not json" }), opts).paramsSchema).toBeTruthy();
  });

  it("params_schema musi być obiektem, nie tablicą ani skalarem", () => {
    expect(validate(validForm({ paramsSchema: "[1]" }), opts).paramsSchema).toBeTruthy();
    expect(validate(validForm({ paramsSchema: "5" }), opts).paramsSchema).toBeTruthy();
    expect(validate(validForm({ paramsSchema: "null" }), opts).paramsSchema).toBeTruthy();
    expect(validate(validForm({ paramsSchema: '{"type":"object"}' }), opts).paramsSchema).toBeUndefined();
  });

  it("nazwa: wymagana, format, zarezerwowane i zajęte (tylko dla nowej akcji)", () => {
    expect(validate(validForm({ name: "" }), opts).name).toBeTruthy();
    expect(validate(validForm({ name: "1abc" }), opts).name).toBeTruthy();
    expect(validate(validForm({ name: "a b" }), opts).name).toBeTruthy();
    for (const reserved of ["escalate", "notify", "http_get", "missing"]) {
      expect(validate(validForm({ name: reserved }), opts).name, reserved).toBeTruthy();
    }
    expect(validate(validForm({ name: "taken" }), { isNew: true, takenNames: new Set(["taken"]) }).name).toBeTruthy();
    expect(validate(validForm({ name: "taken" }), { isNew: false, takenNames: new Set(["taken"]) }).name).toBeUndefined();
  });

  it("timeout: dodatnia liczba całkowita, maksymalnie 300000", () => {
    for (const bad of ["", "abc", "0", "-5", "1.5", "300001"]) {
      expect(validate(validForm({ timeout: bad }), opts).timeout, bad).toBeTruthy();
    }
    expect(validate(validForm({ timeout: "300000" }), opts).timeout).toBeUndefined();
    expect(validate(validForm({ timeout: "1" }), opts).timeout).toBeUndefined();
  });

  it("nagłówki: bez nazwy, zła nazwa, duplikat (bez względu na wielkość liter); puste wiersze ignorowane", () => {
    expect(validate(validForm({ headers: [newRow("", ""), newRow("", "")] }), opts).headers).toBeUndefined();
    expect(validate(validForm({ headers: [newRow("", "v")] }), opts).headers).toBeTruthy();
    expect(validate(validForm({ headers: [newRow("Bad Name", "v")] }), opts).headers).toBeTruthy();
    expect(validate(validForm({ headers: [newRow("X-A", "1"), newRow("x-a", "2")] }), opts).headers).toBeTruthy();
    expect(validate(validForm({ headers: [newRow("X-A", "1"), newRow("X-B", "2")] }), opts).headers).toBeUndefined();
  });
});

describe("looksLikePastedSecret (sekrety tylko jako ${ENV})", () => {
  it("wklejony token Bearer ostrzega", () => {
    expect(looksLikePastedSecret("Authorization", "Bearer sk_live_51H8xYzAbCdEfGhIjKlMnOpQr")).toBe(true);
  });

  it("referencja ${ENV} nie ostrzega", () => {
    expect(looksLikePastedSecret("Authorization", "Bearer ${TOKEN}")).toBe(false);
    expect(looksLikePastedSecret("X-Api-Key", "${CACHE_ADMIN_TOKEN}")).toBe(false);
    expect(looksLikePastedSecret("Authorization", "Bearer ${CACHE_ADMIN_TOKEN}")).toBe(false);
  });

  it("krótka wartość w nagłówku o sekretnej nazwie też ostrzega", () => {
    expect(looksLikePastedSecret("Authorization", "Bearer abc")).toBe(true);
    expect(looksLikePastedSecret("X-API-Key", "hunter2")).toBe(true);
    expect(looksLikePastedSecret("Cookie", "session=abc")).toBe(true);
  });

  it("długi ciąg tokenu ostrzega nawet w niewinnie nazwanym nagłówku", () => {
    expect(looksLikePastedSecret("X-Custom", "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")).toBe(true);
  });

  it("zwykłe nagłówki i krótkie wartości nie ostrzegają", () => {
    expect(looksLikePastedSecret("Content-Type", "application/json")).toBe(false);
    expect(looksLikePastedSecret("Accept", "application/json")).toBe(false);
    expect(looksLikePastedSecret("X-Trace", "abc")).toBe(false);
    expect(looksLikePastedSecret("X-Env", "prod")).toBe(false);
  });

  it("pusta wartość lub sam schemat bez tokenu nie ostrzega", () => {
    expect(looksLikePastedSecret("Authorization", "")).toBe(false);
    expect(looksLikePastedSecret("Authorization", "   ")).toBe(false);
    expect(looksLikePastedSecret("Authorization", "Bearer")).toBe(false);
  });

  it("białe znaki wokół nazwy i wartości nie mają znaczenia", () => {
    expect(looksLikePastedSecret(" Authorization ", " Bearer ${TOKEN} ")).toBe(false);
    expect(looksLikePastedSecret(" Authorization ", " Bearer sk_live_51H8xYzAbCdEfGhIjKlMnOpQr ")).toBe(true);
  });
});
