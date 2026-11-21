// Ustawienia sesji UI: wybrany backend (mock / prawdziwy) oraz uwierzytelniona tożsamość operatora.
// Tożsamość operatora i token sesyjny NIE są zapisywane w localStorage – żyją wyłącznie w pamięci karty
// i są weryfikowane po stronie serwera przez POST /auth/login oraz unieważniane przez POST /auth/logout.
import { useSyncExternalStore } from "react";

export type Backend = "mock" | "api";

export const BACKEND_URLS: Record<Backend, string> = {
  mock: import.meta.env.VITE_MOCK_URL ?? "http://localhost:4010",
  api: import.meta.env.VITE_API_URL ?? "http://localhost:8080",
};

const BACKEND_KEY = "alerting.backend";
const LEGACY_PERSON_KEY = "alerting.personId";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // brak localStorage (tryb prywatny)
  }
}

// Usuń ewentualny stary klucz personId z localStorage po poprzednich wersjach.
write(LEGACY_PERSON_KEY, null);

function isBackend(v: unknown): v is Backend {
  return v === "mock" || v === "api";
}

const envBackend = import.meta.env.VITE_DEFAULT_BACKEND;
const storedBackend = read(BACKEND_KEY);

// Backend wybierany raz przy starcie strony; zmiana przeładowuje UI, żeby nie mieszać danych z dwóch źródeł.
export const backend: Backend = isBackend(storedBackend)
  ? storedBackend
  : isBackend(envBackend)
    ? envBackend
    : "api";

export const API_URL: string = BACKEND_URLS[backend];

export function switchBackend(next: Backend) {
  if (next === backend) return;
  write(BACKEND_KEY, next);
  window.location.reload();
}

// --- uwierzytelniona sesja operatora (wyłącznie w pamięci karty) ---

let personId: string | null = null;
let sessionToken: string | null = null;
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((l) => l());
}

export function getPersonId(): string | null {
  return personId;
}

export function getSessionToken(): string | null {
  return sessionToken;
}

export function setPersonId(id: string | null) {
  personId = id;
  if (id === null) sessionToken = null;
  notify();
}

export async function loginAs(id: string, token?: string): Promise<void> {
  if (backend === "mock") {
    personId = id;
    sessionToken = null;
    notify();
    return;
  }
  const cred = token ?? import.meta.env.VITE_API_TOKEN ?? "dev-token";
  const res = await fetch(`${API_URL}/auth/login`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ person_id: id, token: cred }),
  });
  if (!res.ok) {
    throw new Error(`Logowanie nie powiodło się (${res.status})`);
  }
  const data = (await res.json()) as { person_id: string; session_token?: string };
  personId = data.person_id;
  sessionToken = data.session_token ?? null;
  notify();
}

export async function logout(): Promise<void> {
  const tok = sessionToken;
  personId = null;
  sessionToken = null;
  notify();
  if (backend === "mock") return;
  try {
    await fetch(`${API_URL}/auth/logout`, {
      method: "POST",
      credentials: "include",
      headers: tok ? { Authorization: `Bearer ${tok}` } : {},
    });
  } catch {
    // sesja po stronie klienta już wyczyszczona
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePersonId(): string | null {
  return useSyncExternalStore(subscribe, getPersonId);
}
