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
  }
}

write(LEGACY_PERSON_KEY, null);

function isBackend(v: unknown): v is Backend {
  return v === "mock" || v === "api";
}

const envBackend = import.meta.env.VITE_DEFAULT_BACKEND;
const storedBackend = read(BACKEND_KEY);

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

let personId: string | null = null;
let personName: string | null = null;
let sessionToken: string | null = null;
let authChecked = backend === "mock";
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((l) => l());
}

export function getPersonId(): string | null {
  return personId;
}

export function getPersonName(): string | null {
  return personName;
}

export function getSessionToken(): string | null {
  return sessionToken;
}

export function isAuthChecked(): boolean {
  return authChecked;
}

export function setPersonId(id: string | null) {
  personId = id;
  if (id === null) {
    personName = null;
    sessionToken = null;
  }
  notify();
}

export async function checkSession(): Promise<void> {
  if (backend === "mock") {
    authChecked = true;
    notify();
    return;
  }
  try {
    const res = await fetch(`${API_URL}/auth/me`, {
      method: "GET",
      credentials: "include",
    });
    if (res.ok) {
      const data = (await res.json()) as { person_id?: string | null; authenticated?: boolean };
      if (data.authenticated && data.person_id) {
        personId = data.person_id;
        personName = data.person_id;
      }
    }
  } catch {
  } finally {
    authChecked = true;
    notify();
  }
}

export async function loginWithPassword(username: string, password: string): Promise<void> {
  const cleanUser = username.trim().toLowerCase();
  if (!cleanUser || !password) {
    throw new Error("Please enter both username and password.");
  }
  if (backend === "mock") {
    if (password !== "haslo") {
      throw new Error("Invalid username or password.");
    }
    personId = cleanUser;
    personName = cleanUser;
    sessionToken = null;
    authChecked = true;
    notify();
    return;
  }
  const res = await fetch(`${API_URL}/auth/login`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: cleanUser, password }),
  });
  if (!res.ok) {
    const errBody = (await res.json().catch(() => null)) as { detail?: string; error?: string } | null;
    throw new Error(errBody?.detail || errBody?.error || "Invalid username or password.");
  }
  const data = (await res.json()) as { person_id: string; name?: string; session_token?: string };
  personId = data.person_id;
  personName = data.name ?? data.person_id;
  sessionToken = data.session_token ?? null;
  authChecked = true;
  notify();
}

export async function loginAs(id: string, token?: string): Promise<void> {
  return loginWithPassword(id, token ?? "haslo");
}

export async function logout(): Promise<void> {
  const tok = sessionToken;
  personId = null;
  personName = null;
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
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePersonId(): string | null {
  return useSyncExternalStore(subscribe, getPersonId);
}

export function usePersonName(): string | null {
  return useSyncExternalStore(subscribe, getPersonName);
}

export function useAuthChecked(): boolean {
  return useSyncExternalStore(subscribe, isAuthChecked);
}
