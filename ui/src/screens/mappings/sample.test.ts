import { describe, expect, it } from "vitest";
import { parseSample } from "./sample";

describe("parseSample", () => {
  it("obiekt z API (format json) przechodzi bez zmian", () => {
    const obj = { a: 1 };
    expect(parseSample(obj)).toEqual({ value: obj, format: "json" });
  });

  it("tekst JSON jest parsowany jako json", () => {
    expect(parseSample('{"a": {"b": [1, 2]}}')).toEqual({ value: { a: { b: [1, 2] } }, format: "json" });
  });

  it("tekst YAML jest parsowany do drzewa", () => {
    expect(parseSample("type: WORKFLOW_FAILED\nerror:\n  code: TIMEOUT\n")).toEqual({
      value: { type: "WORKFLOW_FAILED", error: { code: "TIMEOUT" } },
      format: "yaml",
    });
  });

  it("zwykły tekst nie jest obiektem -> invalid z komunikatem", () => {
    const r = parseSample("po prostu tekst");
    expect(r.format).toBe("invalid");
    expect(r.error).toBeTruthy();
  });

  it("zepsuty YAML -> invalid z komunikatem błędu parsera", () => {
    const r = parseSample("a: [1, 2\nb: {");
    expect(r.format).toBe("invalid");
    expect(r.error).toBeTruthy();
  });
});
