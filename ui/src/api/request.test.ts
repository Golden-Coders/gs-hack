import { describe, expect, it } from "vitest";
import { describeError, send } from "./request";
import { actorLabel, formatTime } from "../ui/format";

describe("formatTime", () => {
  it("brak wartości daje półpauzę", () => {
    expect(formatTime(null)).toBe("—");
    expect(formatTime(undefined)).toBe("—");
    expect(formatTime("")).toBe("—");
  });

  it("niepoprawna data wraca bez zmian zamiast „Invalid Date”", () => {
    expect(formatTime("wczoraj")).toBe("wczoraj");
  });

  it("poprawna data jest formatowana lokalnie (pl-PL)", () => {
    const iso = "2026-10-23T09:14:02.331Z";
    expect(formatTime(iso)).toBe(new Date(iso).toLocaleString("pl-PL"));
    expect(formatTime(iso)).toMatch(/2026/);
  });
});

describe("actorLabel", () => {
  it("użytkownik, system, źródło, brak", () => {
    expect(actorLabel({ type: "user", person_id: "anna" })).toBe("anna");
    expect(actorLabel({ type: "user" })).toBe("użytkownik");
    expect(actorLabel({ type: "system" })).toBe("system");
    expect(actorLabel({ type: "source" } as never)).toBe("źródło");
    expect(actorLabel(null)).toBe("—");
  });
});

describe("describeError / send", () => {
  const res = (status: number, ok = status < 400) => ({ status, ok }) as Response;

  it("komunikat z backendu ma pierwszeństwo, szczegół jest dopisany", () => {
    expect(describeError({ error: "conflict", detail: "wersja 2" }, res(409))).toBe("conflict: wersja 2");
  });

  it("bez komunikatu pokazuje status HTTP, bez niczego -> nieznany błąd", () => {
    expect(describeError(undefined, res(500))).toBe("HTTP 500");
    expect(describeError(undefined)).toBe("Nieznany błąd");
  });

  it("send zwraca null przy sukcesie, opis przy błędzie i komunikat przy braku połączenia", async () => {
    expect(await send(async () => ({ response: res(200) }))).toBeNull();
    expect(await send(async () => ({ error: { error: "zły stan" }, response: res(409) }))).toBe("zły stan");
    expect(await send(async () => ({ response: res(502, false) }))).toBe("HTTP 502");
    expect(
      await send(async () => {
        throw new Error("network");
      }),
    ).toBe("Brak połączenia z API");
  });
});
