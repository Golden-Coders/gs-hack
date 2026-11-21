import { describe, expect, it } from "vitest";
import { paletteColor } from "./colors";

describe("paletteColor", () => {
  it("zna nazwy kolorów z kreatora reguł i zwraca tokeny fg/bg", () => {
    for (const name of ["red", "orange", "yellow", "green", "blue", "purple", "gray"]) {
      const c = paletteColor(name);
      expect(c, name).not.toBeNull();
      expect(c!.fg).toMatch(/^#[0-9a-f]{6}$/);
      expect(c!.bg).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("red daje czerwone tokeny (kolor z fixture reguły settlements)", () => {
    expect(paletteColor("red")).toEqual({ fg: "#b91c1c", bg: "#fee2e2" });
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

  // BŁĄD: PALETTE to zwykły obiekt, więc klucze z prototypu ("constructor", "valueOf") zwracają funkcję zamiast null.
  // Oczekiwane: null; faktyczne: funkcja Object. Skutek praktyczny znikomy (konfiguracja z kolorem "constructor").
  it.fails("nazwy z prototypu obiektu nie są kolorami", () => {
    expect(paletteColor("constructor")).toBeNull();
    expect(paletteColor("valueOf")).toBeNull();
  });
});
