import { describe, expect, it } from "vitest";
import { paletteColor } from "./colors";

describe("paletteColor", () => {
  it("zna nazwy kolorów z kreatora reguł i zwraca tokeny fg/bg", () => {
    for (const name of ["red", "orange", "yellow", "green", "blue", "purple", "gray"]) {
      const c = paletteColor(name);
      expect(c, name).not.toBeNull();
      expect(c!.fg).toMatch(/^var\(--[a-z]+(-bg)?\)$/);
      expect(c!.bg).toMatch(/^var\(--[a-z]+(-bg)?\)$/);
    }
  });

  it("red daje czerwone tokeny (kolor z fixture reguły settlements)", () => {
    expect(paletteColor("red")).toEqual({ fg: "var(--red)", bg: "var(--red-bg)" });
  });

  it("ignoruje wielkość liter", () => {
    expect(paletteColor("RED")).toEqual(paletteColor("red"));
    expect(paletteColor("Blue")).toEqual(paletteColor("blue"));
  });

  it("nieznane nazwy dają null (brak wstrzykiwania dowolnego CSS)", () => {
    expect(paletteColor("pink")).toBeNull();
    expect(paletteColor("#ff0000")).toBeNull();
    expect(paletteColor("red; background:url(x)")).toBeNull();
  });

  it("pusta nazwa, null i undefined dają null", () => {
    expect(paletteColor("")).toBeNull();
    expect(paletteColor(null)).toBeNull();
    expect(paletteColor(undefined)).toBeNull();
  });

  it("nazwy z prototypu obiektu nie są kolorami", () => {
    expect(paletteColor("constructor")).toBeNull();
    expect(paletteColor("valueOf")).toBeNull();
  });
});
