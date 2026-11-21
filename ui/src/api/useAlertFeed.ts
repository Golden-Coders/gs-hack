import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Schemas } from "./client";
import { usePollWhenQuiet, useStream } from "./stream";

type Alert = Schemas["AlertSummary"];

export function useAlertFeed() {
  const [alerts, setAlerts] = useState<Map<string, Alert>>(new Map());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [touched, setTouched] = useState<Map<string, number>>(new Map());

  const current = useRef(alerts);
  current.current = alerts;

  const remove = useCallback((alertId: string) => {
    setAlerts((prev) => {
      if (!prev.has(alertId)) return prev;
      const next = new Map(prev);
      next.delete(alertId);
      return next;
    });
    setTouched((prev) => {
      if (!prev.has(alertId)) return prev;
      const next = new Map(prev);
      next.delete(alertId);
      return next;
    });
  }, []);

  const merge = useCallback((items: Alert[], live: boolean) => {
    setAlerts((prev) => {
      const next = new Map(prev);
      items.forEach((a) => {
        if (a.state === "resolved") next.delete(a.id);
        else next.set(a.id, a);
      });
      return next;
    });
    if (live) {
      const now = Date.now();
      setTouched((prev) => {
        const next = new Map(prev);
        items.forEach((a) => {
          if (a.state === "resolved") next.delete(a.id);
          else next.set(a.id, now);
        });
        return next;
      });
    }
  }, []);

  const replaceAll = useCallback((items: Alert[]) => {
    const active = new Map<string, Alert>();
    items.forEach((a) => {
      if (a.state !== "resolved") active.set(a.id, a);
    });
    setAlerts(active);
  }, []);

  const poll = useCallback(async () => {
    const { data } = await api.GET("/alerts");
    if (!data) return;
    const changed = data.filter((a) => {
      const old = current.current.get(a.id);
      if (a.state === "resolved") return Boolean(old);
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
        if (data) replaceAll(data);
        else setError(`Failed to load alerts${error ? `: ${JSON.stringify(error)}` : ""}`);
        setLoading(false);
      },
      () => {
        setError("Cannot connect to backend");
        setLoading(false);
      },
    );
  }, [replaceAll]);
  useEffect(load, [load]);

  useStream((msg) => {
    if (msg.type === "alert_changed") {
      if (msg.data.state === "resolved") remove(msg.data.alert_id);
      else void refetch(msg.data.alert_id);
    } else if (msg.type === "resync") {
      merge(msg.alerts, true);
    } else if (msg.data.kind === "alert_count" && msg.data.alert_id) {
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

  const list = [...alerts.values()]
    .filter((a) => a.state !== "resolved")
    .sort((a, b) => Date.parse(b.last_seen) - Date.parse(a.last_seen));
  return { alerts: list, touched, loading, error, retry: load, remove };
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
