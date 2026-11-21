// Strumień na żywo: SSE `/stream` z ponownym łączeniem i dociąganiem zmian z przerwy.
// Jedno połączenie na kartę, otwierane przy pierwszym subskrybencie.
// Po ponownym połączeniu (`hello`) dociągamy `GET /alerts?since=<ostatni at>` i wysyłamy je jako `resync`.
import { useEffect, useRef, useSyncExternalStore } from "react";
import { api, API_URL, type Schemas } from "./client";
import { backend } from "./session";

export type StreamMessage =
  | { type: "alert_changed"; data: Schemas["AlertChanged"] }
  | { type: "ui_event"; data: Schemas["UiEvent"] }
  | { type: "resync"; alerts: Schemas["AlertSummary"][] };

export type StreamStatus = "idle" | "connecting" | "live" | "reconnecting" | "unavailable";

const RETRY_MIN_MS = 1_000;
const RETRY_MAX_MS = 10_000;
// Zapas na różnicę zegarów klient/serwer przy pierwszym połączeniu (zanim dostaniemy `at` z serwera).
const CLOCK_SKEW_MS = 5_000;

let source: EventSource | null = null;
let status: StreamStatus = "idle";
let retryMs = RETRY_MIN_MS;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
let connectedOnce = false;
let lastAt: string | null = null;

const listeners = new Set<(msg: StreamMessage) => void>();
const statusListeners = new Set<() => void>();

function setStatus(next: StreamStatus) {
  status = next;
  statusListeners.forEach((l) => l());
}

// Czas ostatniej wiadomości ze strumienia – do decyzji, czy dociągać dane zapytaniami.
let lastMessageAt = 0;

function emit(msg: StreamMessage) {
  lastMessageAt = Date.now();
  listeners.forEach((l) => l(msg));
}

function noteAt(at: string) {
  if (!lastAt || Date.parse(at) > Date.parse(lastAt)) lastAt = at;
}

async function resync(since: string) {
  const { data, error } = await api.GET("/alerts", { params: { query: { since } } });
  if (error || !data) return; // przy kolejnym zerwaniu spróbujemy znowu od tego samego `lastAt`
  data.forEach((a) => noteAt(a.last_seen));
  emit({ type: "resync", alerts: data });
}

function parse<T>(e: MessageEvent): T | null {
  try {
    return JSON.parse(e.data) as T;
  } catch {
    return null;
  }
}

function connect() {
  // Prism nie obsługuje SSE – na mocku nie łączymy się w kółko, tylko mówimy to wprost.
  if (backend === "mock") return setStatus("unavailable");
  setStatus(connectedOnce ? "reconnecting" : "connecting");
  const es = new EventSource(`${API_URL}/stream?token=${encodeURIComponent(import.meta.env.VITE_API_TOKEN ?? "dev-token")}`);
  source = es;

  es.addEventListener("hello", () => {
    retryMs = RETRY_MIN_MS;
    setStatus("live");
    if (connectedOnce && lastAt) void resync(lastAt);
    else lastAt ??= new Date(Date.now() - CLOCK_SKEW_MS).toISOString();
    connectedOnce = true;
  });

  es.addEventListener("alert_changed", (e) => {
    const data = parse<Schemas["AlertChanged"]>(e as MessageEvent);
    if (!data) return;
    noteAt(data.at);
    emit({ type: "alert_changed", data });
  });

  es.addEventListener("ui_event", (e) => {
    const data = parse<Schemas["UiEvent"]>(e as MessageEvent);
    if (!data) return;
    noteAt(data.at);
    emit({ type: "ui_event", data });
  });

  // Ponawiamy sami (z backoffem), bo EventSource po błędzie HTTP zamyka się na stałe.
  es.onerror = () => {
    es.close();
    if (source !== es) return;
    source = null;
    setStatus("reconnecting");
    retryTimer = setTimeout(() => {
      retryTimer = null;
      if (listeners.size) connect();
    }, retryMs);
    retryMs = Math.min(retryMs * 2, RETRY_MAX_MS);
  };
}

function disconnect() {
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = null;
  source?.close();
  source = null;
  setStatus("idle");
}

export function subscribe(listener: (msg: StreamMessage) => void): () => void {
  listeners.add(listener);
  if (!source && !retryTimer) connect();
  return () => {
    listeners.delete(listener);
    if (!listeners.size) disconnect();
  };
}

/** Subskrypcja strumienia na czas życia komponentu; listener może się zmieniać między renderami. */
export function useStream(listener: (msg: StreamMessage) => void) {
  const ref = useRef(listener);
  ref.current = listener;
  useEffect(() => subscribe((msg) => ref.current(msg)), []);
}

const QUIET_MS = 15_000;

/**
 * Zabezpieczenie na brak zdarzeń na żywo: co `intervalMs` woła `poll`, jeśli strumień nie jest połączony
 * albo od `QUIET_MS` nic z niego nie przyszło. Nie działa na mocku (dane statyczne) ani w ukrytej karcie.
 */
export function usePollWhenQuiet(poll: () => void, intervalMs = 5_000) {
  const ref = useRef(poll);
  ref.current = poll;
  useEffect(() => {
    if (backend === "mock") return;
    const t = setInterval(() => {
      if (document.hidden) return;
      if (status === "live" && Date.now() - lastMessageAt < QUIET_MS) return;
      ref.current();
    }, intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
}

export function useStreamStatus(): StreamStatus {
  return useSyncExternalStore(
    (l) => {
      statusListeners.add(l);
      return () => statusListeners.delete(l);
    },
    () => status,
  );
}
