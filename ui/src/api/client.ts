// Klient API generowany z contracts/openapi.yaml (typy: `npm run gen:api`).
// Adres zależy od wybranego backendu (mock Prism :4010 albo gateway api :8080) – patrz session.ts.
import createClient from "openapi-fetch";
import type { paths, components } from "./schema";
import { API_URL, getPersonId, getSessionToken } from "./session";

export type Schemas = components["schemas"];

export { API_URL };

export const api = createClient<paths>({
  baseUrl: API_URL,
  credentials: "include",
});

// Używaj serwerowego tokenu sesyjnego operatora (gdy zalogowany) zamiast gołego tokenu współdzielonego.
api.use({
  onRequest({ request }) {
    const sess = getSessionToken();
    const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
    if (!request.headers.has("Authorization")) {
      request.headers.set("Authorization", `Bearer ${bearer}`);
    }
    const id = getPersonId();
    if (id && !request.headers.has("X-Person-Id")) {
      request.headers.set("X-Person-Id", id);
    }
    return request;
  },
});

/** Nagłówek dla operacji, w których kontrakt OpenAPI wymaga pola `X-Person-Id` (serwer weryfikuje je względem sesji). */
export function asPerson(): { "X-Person-Id": string } {
  return { "X-Person-Id": getPersonId() ?? "" };
}
