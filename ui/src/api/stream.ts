// Strumień na żywo: SSE `/stream` z ponownym łączeniem i dociąganiem zmian z przerwy.
// Jedno połączenie na kartę, otwierane przy pierwszym subskrybencie.
// Po ponownym połączeniu (`hello`) dociągamy `GET /alerts?since=<ostatni at>` i wysyłamy je jako `resync`.
import { useEffect, useRef, useSyncExternalStore } from "react";
import { api, API_URL, type Schemas } from "./client";

export type StreamMessage =
  | { type: "alert_changed"; data: Schemas["AlertChanged"] }
  | { type: "ui_event"; data: Schemas["UiEvent"] }
  | { type: "resync"; alerts: Schemas["AlertSummary"][] };

export type StreamStatus = "idle" | "connecting" | "live" | "reconnecting";

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

function emit(msg: StreamMessage) {
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
  setStatus(connectedOnce ? "reconnecting" : "connecting");
  const es = new EventSource(`${API_URL}/stream`);
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

export function useStreamStatus(): StreamStatus {
  return useSyncExternalStore(
    (l) => {
      statusListeners.add(l);
      return () => statusListeners.delete(l);
    },
    () => status,
  );
}
