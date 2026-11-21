import type { Schemas } from "./client";

/** Opis błędu HTTP dla człowieka: komunikat z backendu (`Error.error`) albo sam status. */
export function describeError(error: unknown, response?: Response): string {
  const e = error as Partial<Schemas["Error"]> | undefined;
  const msg = e && typeof e.error === "string" ? e.error : null;
  const detail = e && typeof e.detail === "string" ? e.detail : null;
  const status = response ? `HTTP ${response.status}` : null;
  return [msg ?? status ?? "Nieznany błąd", detail].filter(Boolean).join(": ");
}

/** Wykonuje żądanie zmieniające stan; zwraca komunikat błędu albo `null`, gdy backend przyjął komendę. */
export async function send(request: () => Promise<{ error?: unknown; response: Response }>): Promise<string | null> {
  try {
    const { error, response } = await request();
    return error !== undefined || !response.ok ? describeError(error, response) : null;
  } catch {
    return "Brak połączenia z API";
  }
}
