import { describe, expect, it } from "vitest";
import rates from "../../../../crates/contracts/fixtures/config/rule.rates.json";
import settlements from "../../../../crates/contracts/fixtures/config/rule.settlements.json";
import {
  conditionToNode,
  conditionToRoot,
  emptyForm,
  findLeaves,
  formToRule,
  guessSchema,
  mapTree,
  newGroup,
  newLeaf,
  nodeToCondition,
  parseRuleYaml,
  ruleToForm,
  ruleToYaml,
  skeletonFromFields,
  slugify,
  textToValue,
  validateForm,
  valueToText,
  type Cond,
  type Rule,
  type RuleForm,
  type TreeNode,
} from "./model";

const fixtures: [string, Rule][] = [
  ["rule.settlements", settlements as unknown as Rule],
  ["rule.rates", rates as unknown as Rule],
];

function validForm(patch: Partial<RuleForm> = {}): RuleForm {
  return {
    ...emptyForm(),
    id: "my-rule",
    name: "Moja reguła",
    recipients: [{ type: "team", id: "rates" }],
    ...patch,
  };
}

const leaf = (field: string, op: Cond["op"], value?: unknown): Cond => ({ field, op, ...(value !== undefined ? { value } : {}) });

describe("konwersje reguła <-> kreator", () => {
  it.each(fixtures)("round-trip %s: reguła -> formularz -> reguła daje to samo", (_n, rule) => {
    // `severity_override: null` celowo znika (brak pola = brak nadpisania), reszta bez zmian
    const { severity_override, ...rest } = rule;
    const expected = severity_override ? rule : rest;
    expect(formToRule(ruleToForm(rule))).toEqual(expected);
  });

  it("pomija severity_override, gdy go brak (nie wysyła null)", () => {
    const out = formToRule(ruleToForm(rates as unknown as Rule));
    expect(out).not.toHaveProperty("severity_override");
    expect(formToRule(validForm({ severityOverride: "" }))).not.toHaveProperty("severity_override");
  });

  it("zachowuje severity_override, gdy ustawione", () => {
    expect(formToRule(ruleToForm(settlements as unknown as Rule)).severity_override).toBe("critical");
    expect(formToRule(validForm({ severityOverride: "error" })).severity_override).toBe("error");
  });

  it("wyróżnione pola spoza listy fields trafiają do wierszy i wracają jako highlight", () => {
    const rule: Rule = {
      ...(rates as unknown as Rule),
      presentation: { fields: ["raw.a"], highlight: { "raw.a": "red", "raw.b": "blue" } },
    };
    const form = ruleToForm(rule);
    expect(form.rows.map((r) => [r.path, r.color])).toEqual([["raw.a", "red"], ["raw.b", "blue"]]);
    const back = formToRule(form);
    expect(back.presentation?.highlight).toEqual({ "raw.a": "red", "raw.b": "blue" });
    expect(back.presentation?.fields).toEqual(["raw.a", "raw.b"]);
  });

  it("puste wiersze pól są pomijane, a puste teksty prezentacji stają się null", () => {
    const rule = formToRule(validForm({ rows: [{ uid: "x", path: "  ", color: "red" }], title: "", body: "", color: "" }));
    expect(rule.presentation).toEqual({ title: null, body: null, fields: [], highlight: {}, color: null });
    expect(rule.team).toBeNull();
  });

  it("domyślne wartości dla brakujących pól reguły", () => {
    const f = ruleToForm({ id: "a", name: "A", condition: {}, recipients: [] } as unknown as Rule);
    expect(f).toMatchObject({ enabled: true, version: 0, severityOverride: "", team: "", title: "", color: "" });
  });
});

describe("drzewo warunków", () => {
  it("zagnieżdżone all/any/not: round-trip warunku", () => {
    const c: Cond = {
      all: [
        leaf("schema", "glob", "trade.rejected.*"),
        { any: [leaf("raw.desk", "eq", "rates"), { not: leaf("raw.env", "eq", "dev") }] },
        { not: { all: [leaf("raw.n", "lt", 5), leaf("raw.x", "exists")] } },
      ],
    };
    const root = conditionToRoot(c);
    expect(root.kind).toBe("all");
    expect(nodeToCondition(root)).toEqual(c);
  });

  it("korzeń any zostaje any", () => {
    const c: Cond = { any: [leaf("raw.a", "eq", "1"), leaf("raw.b", "eq", "2")] };
    const root = conditionToRoot(c);
    expect(root.kind).toBe("any");
    expect(nodeToCondition(root)).toEqual(c);
  });

  it("liść w korzeniu jest opakowany w grupę all, bez zmiany znaczenia", () => {
    const root = conditionToRoot(leaf("raw.a", "eq", "x"));
    expect(root.kind).toBe("all");
    expect(root.children).toHaveLength(1);
    expect(nodeToCondition(root)).toEqual({ all: [leaf("raw.a", "eq", "x")] });
  });

  it("brak warunku -> pusta grupa all", () => {
    const root = conditionToRoot(null);
    expect(root.kind).toBe("all");
    expect(root.children).toEqual([]);
  });

  it("warunek z kilkoma kluczami w jednym obiekcie to jawna koniunkcja", () => {
    const n = conditionToNode({ field: "raw.a", op: "eq", value: 1, not: leaf("raw.b", "eq", 2) });
    expect(n.kind).toBe("all");
    if (n.kind === "all") expect(n.children.map((c) => c.kind).sort()).toEqual(["leaf", "not"]);
  });

  it("liść bez op ma domyślnie eq", () => {
    const n = conditionToNode({ field: "raw.a", value: "x" });
    expect(n.kind === "leaf" && n.op).toBe("eq");
  });

  it("pusta negacja serializuje się do pustego all (nie gubi się)", () => {
    expect(nodeToCondition({ uid: "n", kind: "not", child: null })).toEqual({ not: { all: [] } });
  });

  it("mapTree zamienia, usuwa i nie zmienia referencji bez trafienia", () => {
    const a = newLeaf();
    const b = newLeaf();
    const root = { ...newGroup("all"), children: [a, b] };
    expect(mapTree(root, "brak", (n) => n)).toBe(root);
    const removed = mapTree(root, a.uid, () => null) as typeof root;
    expect(removed.children).toEqual([b]);
    const replaced = mapTree(root, b.uid, (n) => ({ ...(n as typeof b), field: "raw.z" })) as typeof root;
    expect((replaced.children[1] as typeof b).field).toBe("raw.z");
  });

  it("findLeaves zbiera liście z not i zagnieżdżonych grup", () => {
    const root = conditionToRoot({ all: [leaf("a", "eq", 1), { not: leaf("b", "eq", 2) }, { any: [leaf("c", "eq", 3)] }] });
    expect(findLeaves(root).map((l) => l.field)).toEqual(["a", "b", "c"]);
  });
});

describe("typy wartości", () => {
  it("operatory liczbowe dają liczby", () => {
    for (const op of ["gt", "gte", "lt", "lte"] as const) expect(textToValue(op, "10000000")).toBe(10000000);
    expect(textToValue("gt", " 2.5 ")).toBe(2.5);
    expect(textToValue("gt", "abc")).toBe("abc");
    expect(textToValue("gt", "")).toBe("");
  });

  it("eq/ne rozpoznają liczby i bool, reszta zostaje tekstem", () => {
    expect(textToValue("eq", "42")).toBe(42);
    expect(textToValue("eq", "true")).toBe(true);
    expect(textToValue("ne", "false")).toBe(false);
    expect(textToValue("eq", "rates")).toBe("rates");
    expect(textToValue("eq", "007")).toBe("007");
    expect(textToValue("eq", "")).toBe("");
  });

  it("contains/regex/glob zawsze tekst", () => {
    expect(textToValue("contains", "123")).toBe("123");
    expect(textToValue("regex", "true")).toBe("true");
    expect(textToValue("glob", "trade.*")).toBe("trade.*");
  });

  it("in daje listę z przycinaniem, pustych pozycji nie ma, elementy mają typy", () => {
    expect(textToValue("in", "TIMEOUT, REJECTED")).toEqual(["TIMEOUT", "REJECTED"]);
    expect(textToValue("in", "1, 2 ,, true")).toEqual([1, 2, true]);
    expect(textToValue("in", "")).toEqual([]);
  });

  it("exists nie ma wartości", () => {
    expect(textToValue("exists", "cokolwiek")).toBeUndefined();
    expect(nodeToCondition({ uid: "n", kind: "leaf", field: "raw.a", op: "exists", value: "" })).toEqual({ field: "raw.a", op: "exists" });
  });

  it("niezmieniona wartość oryginalna zachowuje typ (raw)", () => {
    expect(textToValue("eq", "true", true)).toBe(true);
    expect(textToValue("eq", "1", "1")).toBe("1"); // tekst "1" nie zamienia się w liczbę
    expect(textToValue("in", "A, B", ["A", "B"])).toEqual(["A", "B"]);
    expect(textToValue("eq", "2", 1)).toBe(2); // zmiana -> nowa wartość
  });

  it("round-trip: liczby, bool, listy i tekst-liczba przechodzą bez zmiany typu", () => {
    const c: Cond = {
      all: [
        leaf("raw.n", "gt", 10000000),
        leaf("raw.flag", "eq", true),
        leaf("raw.s", "eq", "1"),
        leaf("raw.code", "in", ["TIMEOUT", "REJECTED"]),
        leaf("raw.nums", "in", [1, 2, 3]),
      ],
    };
    expect(nodeToCondition(conditionToRoot(c))).toEqual(c);
  });

  it("valueToText", () => {
    expect(valueToText(null)).toBe("");
    expect(valueToText(undefined)).toBe("");
    expect(valueToText("a")).toBe("a");
    expect(valueToText(5)).toBe("5");
    expect(valueToText(false)).toBe("false");
    expect(valueToText(["a", 1])).toBe("a, 1");
  });
});

describe("validateForm", () => {
  it("poprawny formularz nie ma błędów", () => {
    const f = validForm({ root: conditionToRoot({ all: [leaf("raw.a", "eq", "x")] }) });
    expect(validateForm(f, true).count).toBe(0);
  });

  it("fixtures przechodzą walidację", () => {
    for (const [name, r] of fixtures) expect(validateForm(ruleToForm(r), false).count, name).toBe(0);
  });

  it("wymaga nazwy, id i odbiorcy", () => {
    const e = validateForm(emptyForm(), true);
    expect(e.name && e.id && e.recipients).toBeTruthy();
    expect(e.count).toBe(3);
  });

  it("id: format sprawdzany tylko dla nowej reguły", () => {
    expect(validateForm(validForm({ id: "Zły Id" }), true).id).toBeTruthy();
    expect(validateForm(validForm({ id: "Zły Id" }), false).id).toBeUndefined();
  });

  it("liść: brak pola i wartości; exists nie wymaga wartości", () => {
    const l = newLeaf();
    const e = validateForm(validForm({ root: { ...newGroup("all"), children: [l] } }), true);
    expect(e.leaves[l.uid]).toMatchObject({ field: expect.any(String), value: expect.any(String) });
    const ex: TreeNode = { ...newLeaf(), field: "raw.a", op: "exists" };
    expect(validateForm(validForm({ root: { ...newGroup("all"), children: [ex] } }), true).count).toBe(0);
  });

  it("liczba: tekst w operatorze liczbowym to błąd formatu", () => {
    const l: TreeNode = { ...newLeaf(), field: "raw.n", op: "gt", value: "dużo" };
    const e = validateForm(validForm({ root: { ...newGroup("all"), children: [l] } }), true);
    expect(e.leaves[l.uid].kind).toBe("format");
  });

  it("regex: niepoprawne wyrażenie to błąd formatu; in: same przecinki to brak wartości", () => {
    const r: TreeNode = { ...newLeaf(), field: "raw.a", op: "regex", value: "(" };
    const i: TreeNode = { ...newLeaf(), field: "raw.a", op: "in", value: " , " };
    const e = validateForm(validForm({ root: { ...newGroup("all"), children: [r, i] } }), true);
    expect(e.leaves[r.uid].kind).toBe("format");
    expect(e.leaves[i.uid]).toBeDefined();
  });

  it("negacja bez warunku to błąd, pusta grupa to tylko uwaga", () => {
    const n: TreeNode = { uid: "nn", kind: "not", child: null };
    const g = newGroup("any");
    const e = validateForm(validForm({ root: { ...newGroup("all"), children: [n, g] } }), true);
    expect(e.groups["nn"]).toBeTruthy();
    expect(e.warnings[g.uid]).toBeTruthy();
    expect(e.count).toBe(1);
  });

  it("szablony: niedomknięte {{ }} w tytule i treści", () => {
    const e = validateForm(validForm({ title: "x {{raw.a", body: "y }}" }), true);
    expect(e.title && e.body).toBeTruthy();
    expect(validateForm(validForm({ title: "{{raw.a}} i {{raw.b}}" }), true).title).toBeUndefined();
  });
});

describe("YAML", () => {
  it.each(fixtures)("round-trip %s: reguła -> YAML -> reguła daje to samo", (_n, rule) => {
    const res = parseRuleYaml(ruleToYaml(rule), rule.version ?? 0);
    expect(res).toEqual({ ok: true, rule });
  });

  it("YAML nie zawiera severity_override ani pustych pól prezentacji, gdy ich brak", () => {
    const y = ruleToYaml(rates as unknown as Rule);
    expect(y).not.toContain("severity_override");
    expect(y).not.toContain("highlight");
    expect(y).not.toContain("body");
    expect(ruleToYaml(settlements as unknown as Rule)).toContain("severity_override: critical");
  });

  it("YAML nie zawiera wersji; przy parsowaniu bierze poprzednią wersję", () => {
    const y = ruleToYaml(settlements as unknown as Rule);
    expect(y).not.toMatch(/^version:/m);
    const res = parseRuleYaml(y, 7);
    expect(res.ok && res.rule.version).toBe(7);
  });

  it("kreator -> YAML -> kreator zachowuje warunek (zagnieżdżenia i typy)", () => {
    const c: Cond = { all: [leaf("raw.n", "gt", 5), { any: [leaf("raw.c", "in", ["A", "B"]), { not: leaf("raw.f", "eq", false) }] }] };
    const rule = formToRule(validForm({ root: conditionToRoot(c) }));
    const res = parseRuleYaml(ruleToYaml(rule), 0);
    expect(res.ok).toBe(true);
    if (res.ok) expect(nodeToCondition(ruleToForm(res.rule).root)).toEqual(c);
  });

  it("składnia YAML: błąd z numerem linii", () => {
    const res = parseRuleYaml("id: a\nname: B\ncondition: {all: [\nrecipients: []\n", 0);
    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.line).toBeGreaterThanOrEqual(1);
      expect(res.message).not.toBe("");
    }
  });

  it("pusty tekst", () => {
    expect(parseRuleYaml("   \n", 0)).toEqual({ ok: false, message: "YAML jest pusty" });
  });

  it("nieznany operator: komunikat z nazwą operatora i linią tego warunku", () => {
    const yaml = [
      "id: r1",
      "name: R",
      "condition:",
      "  all:",
      "    - field: raw.a",
      "      op: equals",
      "      value: x",
      "recipients:",
      "  - team:rates",
      "",
    ].join("\n");
    const res = parseRuleYaml(yaml, 0);
    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.message).toContain("equals");
      expect(res.message).toMatch(/operator/);
      expect(res.line).toBe(6);
    }
  });

  it("operator bez wartości (poza exists) to błąd z linią", () => {
    const yaml = "id: r1\nname: R\ncondition:\n  field: raw.a\n  op: eq\nrecipients: []\n";
    const res = parseRuleYaml(yaml, 0);
    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.message).toMatch(/wymaga wartości/);
      expect(res.line).toBeGreaterThanOrEqual(3);
    }
    expect(parseRuleYaml("id: r1\nname: R\ncondition:\n  field: raw.a\n  op: exists\nrecipients: []\n", 0).ok).toBe(true);
  });

  it("nieznany klucz, brak wymaganych pól, zły odbiorca, zły kolor, zły severity", () => {
    const bad = (yaml: string, re: RegExp) => {
      const r = parseRuleYaml(yaml, 0);
      expect(r.ok, yaml).toBe(false);
      if (!r.ok) expect(r.message).toMatch(re);
    };
    const head = "id: r\nname: N\ncondition: {field: raw.a, op: eq, value: 1}\n";
    bad(head + "recipients: []\nfoo: 1\n", /nieznany klucz/);
    bad("name: N\ncondition: {}\nrecipients: []\n", /id/);
    bad("id: r\ncondition: {}\nrecipients: []\n", /name/);
    bad("id: r\nname: N\nrecipients: []\n", /condition/);
    bad("id: r\nname: N\ncondition: {}\n", /recipients/);
    bad(head + "recipients: [{type: robot, id: x}]\n", /person albo team/);
    bad(head + "recipients: [rates]\n", /person:<id>/);
    bad(head + "recipients: []\npresentation: {color: pink}\n", /kolor/);
    bad(head + "recipients: []\nseverity_override: fatal\n", /severity/);
    bad(head + "recipients: []\nenabled: yes please\n", /enabled/);
  });

  it("odbiorcy zapisani skrótem person:/team: są akceptowani", () => {
    const r = parseRuleYaml("id: r\nname: N\ncondition: {field: raw.a, op: exists}\nrecipients: ['person:anna', 'team:rates']\n", 0);
    expect(r.ok && r.rule.recipients).toEqual([
      { type: "person", id: "anna" },
      { type: "team", id: "rates" },
    ]);
  });

  it("brak op oznacza eq", () => {
    const r = parseRuleYaml("id: r\nname: N\ncondition: {field: raw.a, value: 1}\nrecipients: []\n", 0);
    expect(r.ok && r.rule.condition).toEqual({ field: "raw.a", op: "eq", value: 1 });
  });
});

describe("pomocnicze", () => {
  it("slugify transliteruje polskie znaki", () => {
    expect(slugify("Rozrachunki: błąd walidacji!")).toBe("rozrachunki-blad-walidacji");
    expect(slugify("  Żółć  ")).toBe("zolc");
    expect(slugify("---")).toBe("");
  });

  it("guessSchema bierze schemat z liścia schema = <wartość>", () => {
    expect(guessSchema(conditionToRoot({ all: [leaf("schema", "eq", " workflow.v1 ")] }))).toBe("workflow.v1");
    expect(guessSchema(conditionToRoot({ all: [leaf("schema", "glob", "trade.*")] }))).toBe("");
    expect(guessSchema(conditionToRoot(null))).toBe("");
  });

  it("skeletonFromFields buduje zagnieżdżony szkielet zdarzenia", () => {
    expect(skeletonFromFields(["raw.payload.desk", "raw.payload.tradeId", "severity"])).toEqual({
      raw: { payload: { desk: "", tradeId: "" } },
      severity: "",
    });
  });
});
