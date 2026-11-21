import type { Schemas } from "./client";

/** Opis błędu HTTP dla człowieka: komunikat z backendu (`Error.error`) albo sam status. */
export function describeError(error: unknown, response?: Response): string {
  const e = error as Partial<Schemas["Error"]> | undefined;
  const msg = e && typeof e.error === "string" ? e.error : null;
  const detail = e && typeof e.detail === "string" ? e.detail : null;
  const status = response ? `HTTP ${response.status}` : null;
  return [msg ?? status ?? "Unknown error", detail].filter(Boolean).join(": ");
}

/** Sends a state-mutating request; returns an error message or `null` if accepted. */
export async function send(request: () => Promise<{ error?: unknown; response: Response }>): Promise<string | null> {
  try {
    const { error, response } = await request();
    return error !== undefined || !response.ok ? describeError(error, response) : null;
  } catch {
    return "Cannot connect to API";
  }
}
