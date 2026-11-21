// Lista alertów na żywo: stan początkowy z `GET /alerts`, potem zmiany ze strumienia.
// - alert_changed → pełne podsumowanie z `GET /alerts/{id}` (zdarzenie nie ma tytułu, zespołów ani severity po regule)
// - ui_event alert_count → licznik i ostatnie wystąpienie w miejscu (bez zapytania, bo bywa ich setki)
// - resync → scalenie alertów dociągniętych po przerwie
// - gdy strumień milczy → okresowe GET /alerts (usePollWhenQuiet)
import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Schemas } from "./client";
import { usePollWhenQuiet, useStream } from "./stream";

type Alert = Schemas["AlertSummary"];

export function useAlertFeed() {
  const [alerts, setAlerts] = useState<Map<string, Alert>>(new Map());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // id → czas ostatniej zmiany na żywo, do krótkiego wyróżnienia wiersza
  const [touched, setTouched] = useState<Map<string, number>>(new Map());

  // Kopia stanu do porównań poza setState (dociąganie wyróżnia tylko faktycznie zmienione alerty).
  const current = useRef(alerts);
  current.current = alerts;

  const merge = useCallback((items: Alert[], live: boolean) => {
    setAlerts((prev) => {
      const next = new Map(prev);
      items.forEach((a) => next.set(a.id, a));
      return next;
    });
    if (live) {
      const now = Date.now();
      setTouched((prev) => {
        const next = new Map(prev);
        items.forEach((a) => next.set(a.id, now));
        return next;
      });
    }
  }, []);

  const poll = useCallback(async () => {
    const { data } = await api.GET("/alerts");
    if (!data) return;
    const changed = data.filter((a) => {
      const old = current.current.get(a.id);
      return !old || old.last_seen !== a.last_seen || old.state !== a.state || old.count !== a.count;
    });
    if (changed.length) merge(changed, true);
  }, [merge]);
  usePollWhenQuiet(() => void poll());

  const refetch = useCallback(
    async (alertId: string) => {
      const { data } = await api.GET("/alerts/{alert_id}", { params: { path: { alert_id: alertId } } });
      if (data) merge([data], true);
    },
    [merge],
  );

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    api.GET("/alerts").then(
      ({ data, error }) => {
        if (data) merge(data, false);
        else setError(`Nie udało się pobrać alertów${error ? `: ${JSON.stringify(error)}` : ""}`);
        setLoading(false);
      },
      () => {
        setError("Brak połączenia z backendem");
        setLoading(false);
      },
    );
  }, [merge]);
  useEffect(load, [load]);

  useStream((msg) => {
    if (msg.type === "alert_changed") void refetch(msg.data.alert_id);
    else if (msg.type === "resync") merge(msg.alerts, true);
    else if (msg.data.kind === "alert_count" && msg.data.alert_id) {
      const id = msg.data.alert_id;
      const p = msg.data.payload as { count?: number; last_seen?: string } | null;
      setAlerts((prev) => {
        const a = prev.get(id);
        if (!a || typeof p?.count !== "number") return prev;
        const next = new Map(prev);
        next.set(id, { ...a, count: p.count, last_seen: p.last_seen ?? a.last_seen });
        return next;
      });
      setTouched((prev) => new Map(prev).set(id, Date.now()));
    }
  });

  const list = [...alerts.values()].sort((a, b) => Date.parse(b.last_seen) - Date.parse(a.last_seen));
  return { alerts: list, touched, loading, error, retry: load };
}

/** Zespoły, do których należy osoba (z `GET /teams`). `null` do czasu pobrania. */
export function useTeamsOf(personId: string | null): string[] | null {
  const [teams, setTeams] = useState<Schemas["TeamDef"][] | null>(null);
  useEffect(() => {
    api.GET("/teams").then(({ data }) => setTeams(data ?? []), () => setTeams([]));
  }, []);
  if (!teams) return null;
  return personId ? teams.filter((t) => t.members.includes(personId)).map((t) => t.id) : [];
}
