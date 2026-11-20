// Ustawienia sesji UI: wybrany backend (mock / prawdziwy) i tożsamość („działam jako”).
// Trzymane w localStorage – to wygoda per przeglądarka; bez niego UI działa na wartościach domyślnych.
import { useSyncExternalStore } from "react";

export type Backend = "mock" | "api";

export const BACKEND_URLS: Record<Backend, string> = {
  mock: import.meta.env.VITE_MOCK_URL ?? "http://localhost:4010",
  api: import.meta.env.VITE_API_URL ?? "http://localhost:8080",
};

const BACKEND_KEY = "alerting.backend";
const PERSON_KEY = "alerting.personId";

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
    // brak localStorage (tryb prywatny) – ustawienie żyje tylko do przeładowania
  }
}

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

// --- tożsamość: mały store obserwowany przez useSyncExternalStore

let personId: string | null = read(PERSON_KEY);
const listeners = new Set<() => void>();

export function getPersonId(): string | null {
  return personId;
}

export function setPersonId(id: string | null) {
  personId = id;
  write(PERSON_KEY, id);
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePersonId(): string | null {
  return useSyncExternalStore(subscribe, getPersonId);
}
