import { describe, expect, it } from "vitest";
import {
  defaultFieldName,
  getAt,
  parseConditionField,
  parseJsonPath,
  pathKey,
  toConditionField,
  toJsonPath,
} from "./paths";

describe("toJsonPath", () => {
  it("składa ścieżkę z kluczy i indeksów", () => {
    expect(toJsonPath([])).toBe("$");
    expect(toJsonPath(["labels", "service"])).toBe("$.labels.service");
    expect(toJsonPath(["items", 0, "id"])).toBe("$.items[0].id");
  });

  it("nietypowe klucze trafiają do nawiasów, apostrof jest escapowany", () => {
    expect(toJsonPath(["a b"])).toBe("$['a b']");
    expect(toJsonPath(["it's"])).toBe("$['it\\'s']");
    expect(toJsonPath(["123"])).toBe("$['123']");
  });

  it("klucze z myślnikiem i podkreśleniem zostają w notacji kropkowej", () => {
    expect(toJsonPath(["x-request-id", "_a"])).toBe("$.x-request-id._a");
  });
});

describe("parseJsonPath", () => {
  it("parsuje zagnieżdżenia i indeksy tablic", () => {
    expect(parseJsonPath("$")).toEqual([]);
    expect(parseJsonPath("$.error.code")).toEqual(["error", "code"]);
    expect(parseJsonPath("$.items[0].id")).toEqual(["items", 0, "id"]);
    expect(parseJsonPath("$.m[1][2]")).toEqual(["m", 1, 2]);
  });

  it("obsługuje klucze w nawiasach (apostrofy i cudzysłowy)", () => {
    expect(parseJsonPath("$['a b']")).toEqual(["a b"]);
    expect(parseJsonPath('$["a b"].c')).toEqual(["a b", "c"]);
  });

  it("toleruje białe znaki wokół ścieżki", () => {
    expect(parseJsonPath("  $.a.b  ")).toEqual(["a", "b"]);
  });

  it("składnia zaawansowana lub błędna daje null", () => {
    expect(parseJsonPath("labels.service")).toBeNull();
    expect(parseJsonPath("")).toBeNull();
    expect(parseJsonPath("$.a.")).toBeNull();
    expect(parseJsonPath("$..a")).toBeNull();
    expect(parseJsonPath("$.a[*]")).toBeNull();
    expect(parseJsonPath("$.a[0")).toBeNull();
    expect(parseJsonPath("$.a[?(@.x==1)]")).toBeNull();
  });

  it("round-trip toJsonPath -> parseJsonPath", () => {
    const cases = [[], ["a"], ["labels", "service"], ["items", 0, "id"], ["a b", "c"], ["it's"], ["123"], ["x-y", 3, 4]];
    for (const segs of cases) expect(parseJsonPath(toJsonPath(segs)), JSON.stringify(segs)).toEqual(segs);
  });

  // BŁĄD: parser szuka pierwszego "]" (indexOf), więc klucz zawierający "]" (np. "items[0]") nie przechodzi round-tripu
  // przez toJsonPath. Oczekiwane: ["items[0]"]; faktyczne: null. Rzadkie w praktyce (klucze z nawiasami w payloadzie).
  it.fails("round-trip dla klucza zawierającego nawias zamykający", () => {
    expect(parseJsonPath(toJsonPath(["items[0]"]))).toEqual(["items[0]"]);
  });
});

describe("toConditionField / parseConditionField", () => {
  it("dodaje prefiks raw. i liczy indeksy jako liczby przy parsowaniu", () => {
    expect(toConditionField(["payload", "desk"])).toBe("raw.payload.desk");
    expect(toConditionField(["items", 0, "id"])).toBe("raw.items.0.id");
    expect(parseConditionField("raw.items.0.id")).toEqual(["items", 0, "id"]);
    expect(parseConditionField("raw.payload.notional")).toEqual(["payload", "notional"]);
  });

  it("samo raw to korzeń payloadu", () => {
    expect(parseConditionField("raw")).toEqual([]);
  });

  it("pola kanoniczne (nie raw.) dają null", () => {
    expect(parseConditionField("severity")).toBeNull();
    expect(parseConditionField("schema")).toBeNull();
    expect(parseConditionField("rawx.a")).toBeNull();
    expect(parseConditionField("")).toBeNull();
  });
});

describe("getAt", () => {
  const raw = {
    labels: { service: "payments-api" },
    items: [{ id: "a" }, { id: "b" }],
    nothing: null,
    zero: 0,
    flag: false,
  };

  it("czyta zagnieżdżone pola i elementy tablic", () => {
    expect(getAt(raw, ["labels", "service"])).toBe("payments-api");
    expect(getAt(raw, ["items", 1, "id"])).toBe("b");
    expect(getAt(raw, [])).toBe(raw);
  });

  it("brakujące pola i indeksy dają undefined, bez wyjątku", () => {
    expect(getAt(raw, ["labels", "region"])).toBeUndefined();
    expect(getAt(raw, ["missing", "deep", "path"])).toBeUndefined();
    expect(getAt(raw, ["items", 5, "id"])).toBeUndefined();
  });

  it("przejście przez null lub wartość skalarną daje undefined", () => {
    expect(getAt(raw, ["nothing", "x"])).toBeUndefined();
    expect(getAt(raw, ["labels", "service", "length"])).toBeUndefined();
    expect(getAt(null, ["a"])).toBeUndefined();
    expect(getAt(undefined, ["a"])).toBeUndefined();
  });

  it("zwraca wartości falsy bez zamiany na undefined", () => {
    expect(getAt(raw, ["nothing"])).toBeNull();
    expect(getAt(raw, ["zero"])).toBe(0);
    expect(getAt(raw, ["flag"])).toBe(false);
  });
});

describe("defaultFieldName / pathKey", () => {
  it("nazwa pola to ostatni klucz", () => {
    expect(defaultFieldName(["labels", "service"])).toBe("service");
  });

  it("dla indeksu: klucz nadrzędny + indeks", () => {
    expect(defaultFieldName(["items", 2])).toBe("items2");
    expect(defaultFieldName(["a", "items", 0])).toBe("items0");
  });

  it("bez klucza tekstowego wraca „pole”", () => {
    expect(defaultFieldName([3])).toBe("pole3");
  });

  it("pathKey łączy segmenty kropką", () => {
    expect(pathKey(["a", 0, "b"])).toBe("a.0.b");
  });
});
