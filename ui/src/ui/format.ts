import type { Schemas } from "../api/client";

export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("en-US");
}

export function actorLabel(actor: Schemas["Actor"] | null | undefined): string {
  if (!actor) return "—";
  if (actor.type === "user") return actor.person_id ?? "user";
  return actor.type === "system" ? "system" : "source";
}

/** Relative time ("12s ago"); full date if older than 24 hours. */
export function ago(iso: string, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return formatTime(iso);
}
