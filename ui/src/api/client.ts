// Klient API generowany z contracts/openapi.yaml (typy: `npm run gen:api`).
// Adres: VITE_API_URL (mock Prism :4010 albo gateway api :8080).
import createClient from "openapi-fetch";
import type { paths, components } from "./schema";

export type Schemas = components["schemas"];

export const API_URL: string = import.meta.env.VITE_API_URL ?? "http://localhost:4010";

export const api = createClient<paths>({
  baseUrl: API_URL,
  headers: { Authorization: `Bearer ${import.meta.env.VITE_API_TOKEN ?? "dev-token"}` },
});
