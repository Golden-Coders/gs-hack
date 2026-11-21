// Formatowanie wartości w UI (czasy, autorzy działań).
import type { Schemas } from "../api/client";

export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("pl-PL");
}

export function actorLabel(actor: Schemas["Actor"] | null | undefined): string {
  if (!actor) return "—";
  if (actor.type === "user") return actor.person_id ?? "użytkownik";
  return actor.type === "system" ? "system" : "źródło";
}

/** Czas względny („12 s temu”); starsze niż doba – pełna data. */
export function ago(iso: string, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (s < 60) return `${s} s temu`;
  if (s < 3600) return `${Math.floor(s / 60)} min temu`;
  if (s < 86400) return `${Math.floor(s / 3600)} h temu`;
  return formatTime(iso);
}
