import { describe, expect, it } from "vitest";
import alertmanager from "../../../../crates/contracts/fixtures/config/mapping.alertmanager.json";
import trade from "../../../../crates/contracts/fixtures/config/mapping.trade.json";
import workflow from "../../../../crates/contracts/fixtures/config/mapping.workflow.json";
import { coerceValue, draftFromMapping, emptyDraft, isSimpleLeaf, mappingFromDraft, validate, valueToText, type Draft, type Mapping } from "./draft";

const fixtures: [string, Mapping][] = [
  ["workflow.v1", workflow as unknown as Mapping],
  ["alertmanager", alertmanager as unknown as Mapping],
  ["trade.rejected.v2", trade as unknown as Mapping],
];

describe("draftFromMapping / mappingFromDraft", () => {
  it.each(fixtures)("round-trip mappingu %s zwraca ten sam MappingDef", (_name, m) => {
    expect(mappingFromDraft(draftFromMapping(m))).toEqual(m);
  });

  it("tryb severity path: mapa trafia do wierszy, a z powrotem do obiektu", () => {
    const d = draftFromMapping(workflow as unknown as Mapping);
    expect(d.severity.type).toBe("path");
    if (d.severity.type !== "path") return;
    expect(d.severity.path).toBe("$.severity");
    expect(d.severity.map.map((r) => [r.key, r.value])).toContainEqual(["ERROR", "error"]);
    expect(d.severity.default).toBe("warning");
    expect(d.fields.map((f) => f.name)).toEqual(["type", "workflowName", "workflowId", "step", "environment"]);
  });

  it("tryb severity rules zachowuje warunki i wartości", () => {
    const d = draftFromMapping(trade as unknown as Mapping);
    expect(d.severity.type).toBe("rules");
    if (d.severity.type !== "rules") return;
    expect(d.severity.rules).toHaveLength(1);
    expect(d.severity.rules[0].value).toBe("critical");
    expect(d.severity.rules[0].when).toEqual({ field: "raw.payload.notional", op: "gt", value: 10000000 });
  });

  it("tryb const", () => {
    const m: Mapping = { schema: "x", fields: {}, title: null, severity: { type: "const", value: "error" }, fingerprint: [], resolved: null };
    const d = draftFromMapping(m);
    expect(d.severity).toEqual({ type: "const", value: "error" });
    expect(mappingFromDraft(d).severity).toEqual({ type: "const", value: "error" });
  });

  it("mappingFromDraft przycina białe znaki, pomija puste pola i puste klucze mapy severity", () => {
    const d: Draft = {
      ...emptyDraft("  s1 "),
      title: "  ",
      fields: [
        { id: 1, name: " a ", path: " $.a " },
        { id: 2, name: "  ", path: "$.b" },
      ],
      fingerprint: [" $.a ", "", "  "],
      severity: { type: "path", path: " $.sev ", map: [{ id: 3, key: "", value: "info" }, { id: 4, key: "E", value: "error" }], default: "warning" },
    };
    const m = mappingFromDraft(d);
    expect(m.schema).toBe("s1");
    expect(m.title).toBeNull();
    expect(m.fields).toEqual({ a: "$.a" });
    expect(m.fingerprint).toEqual(["$.a"]);
    expect(m.severity).toEqual({ type: "path", path: "$.sev", map: { E: "error" }, default: "warning" });
  });

  it("wersja jest przenoszona tylko gdy istnieje", () => {
    expect(mappingFromDraft(emptyDraft("x"))).not.toHaveProperty("version");
    expect(mappingFromDraft({ ...emptyDraft("x"), version: 3 }).version).toBe(3);
  });

  it("edycja draftu nie mutuje oryginalnego mappingu", () => {
    const m = JSON.parse(JSON.stringify(workflow)) as Mapping;
    const d = draftFromMapping(m);
    d.fingerprint.push("$.extra");
    expect(m.fingerprint).toHaveLength(3);
  });
});

describe("coerceValue / valueToText / isSimpleLeaf", () => {
  it("operatory liczbowe: liczba z tekstu, a nie-liczba zostaje tekstem", () => {
    expect(coerceValue("10000000", "", "gt")).toBe(10000000);
    expect(coerceValue("abc", 5, "lte")).toBe("abc");
    expect(coerceValue("", 5, "gt")).toBe("");
  });

  it("eq zachowuje typ poprzedniej wartości (liczba, bool), inaczej tekst", () => {
    expect(coerceValue("42", 1, "eq")).toBe(42);
    expect(coerceValue("false", true, "eq")).toBe(false);
    expect(coerceValue("42", "x", "eq")).toBe("42");
    expect(coerceValue("maybe", true, "eq")).toBe("maybe");
  });

  it("valueToText", () => {
    expect(valueToText(undefined)).toBe("");
    expect(valueToText(null)).toBe("");
    expect(valueToText("resolved")).toBe("resolved");
    expect(valueToText(10)).toBe("10");
    expect(valueToText(["a"])).toBe('["a"]');
  });

  it("isSimpleLeaf: liść z prostym operatorem tak; in, grupy i null nie", () => {
    expect(isSimpleLeaf({ field: "raw.a", op: "eq", value: 1 })).toBe(true);
    expect(isSimpleLeaf({ field: "raw.a" })).toBe(true);
    expect(isSimpleLeaf({ field: "raw.a", op: "in", value: [1] })).toBe(false);
    expect(isSimpleLeaf({ all: [] })).toBe(false);
    expect(isSimpleLeaf({ field: "raw.a", op: "eq", not: { field: "b", op: "eq", value: 1 } })).toBe(false);
    expect(isSimpleLeaf(null)).toBe(false);
  });
});

describe("validate", () => {
  it("fixtures są poprawne", () => {
    for (const [name, m] of fixtures) expect(validate(draftFromMapping(m)).any, name).toBe(false);
  });

  it("pusty schemat", () => {
    const p = validate(emptyDraft(" "));
    expect(p.schema).toBeTruthy();
    expect(p.any).toBe(true);
  });

  it("pole bez nazwy, zdublowane i ze ścieżką bez $", () => {
    const d: Draft = {
      ...emptyDraft("s"),
      fields: [
        { id: 1, name: "", path: "$.a" },
        { id: 2, name: "x", path: "$.a" },
        { id: 3, name: "x", path: "$.b" },
        { id: 4, name: "y", path: "a.b" },
      ],
    };
    const p = validate(d);
    expect(p.fields[1]).toMatch(/nazwę/);
    expect(p.fields[2]).toBeUndefined();
    expect(p.fields[3]).toMatch(/powtarza/);
    expect(p.fields[4]).toMatch(/\$/);
  });

  it("tytuł: ścieżka $ albo szablon {{...}}, inaczej błąd", () => {
    expect(validate({ ...emptyDraft("s"), title: "$.error.message" }).title).toBeUndefined();
    expect(validate({ ...emptyDraft("s"), title: "Błąd {{raw.x}}" }).title).toBeUndefined();
    expect(validate({ ...emptyDraft("s"), title: "zwykły tekst" }).title).toBeTruthy();
  });

  it("severity path wymaga ścieżki, rules wymaga co najmniej jednej reguły", () => {
    expect(validate({ ...emptyDraft("s"), severity: { type: "path", path: "severity", map: [], default: "info" } }).severity).toBeTruthy();
    expect(validate({ ...emptyDraft("s"), severity: { type: "rules", rules: [], default: "info" } }).severity).toBeTruthy();
  });

  it("fingerprint: błąd przy ścieżce bez $ pod właściwym indeksem", () => {
    const p = validate({ ...emptyDraft("s"), fingerprint: ["$.a", "b"] });
    expect(p.fingerprint[0]).toBeUndefined();
    expect(p.fingerprint[1]).toBeTruthy();
    expect(p.any).toBe(true);
  });

  it("warunek resolved: pole wymagane, wartość wymagana poza exists", () => {
    const base = emptyDraft("s");
    expect(validate({ ...base, resolved: { field: " ", op: "eq", value: "x" } }).resolved).toBeTruthy();
    expect(validate({ ...base, resolved: { field: "raw.status", op: "eq", value: "" } }).resolved).toBeTruthy();
    expect(validate({ ...base, resolved: { field: "raw.status", op: "exists" } }).resolved).toBeUndefined();
    expect(validate({ ...base, resolved: { field: "raw.status", op: "eq", value: "resolved" } }).resolved).toBeUndefined();
  });
});
