// Klient API generowany z contracts/openapi.yaml (typy: `npm run gen:api`).
// Adres zależy od wybranego backendu (mock Prism :4010 albo gateway api :8080) – patrz session.ts.
import createClient from "openapi-fetch";
import type { paths, components } from "./schema";
import { API_URL, getPersonId } from "./session";

export type Schemas = components["schemas"];

export { API_URL };

export const api = createClient<paths>({
  baseUrl: API_URL,
  headers: { Authorization: `Bearer ${import.meta.env.VITE_API_TOKEN ?? "dev-token"}` },
});

// X-Person-Id dla każdego żądania, jeśli wołający nie podał go sam.
api.use({
  onRequest({ request }) {
    const id = getPersonId();
    if (id && !request.headers.has("X-Person-Id")) request.headers.set("X-Person-Id", id);
    return request;
  },
});

/** Nagłówek dla operacji, w których kontrakt wymaga `X-Person-Id` (typy tego żądają jawnie). */
export function asPerson(): { "X-Person-Id": string } {
  return { "X-Person-Id": getPersonId() ?? "" };
}
