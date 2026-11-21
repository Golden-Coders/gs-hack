// Feed alertów na żywo (tasks.md 14.1): filtr „mój zespół / wszystkie”, stan, severity, licznik, ostatnie wystąpienie.
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { usePersonId } from "../api/session";
import { useStreamStatus } from "../api/stream";
import { useAlertFeed, useTeamsOf } from "../api/useAlertFeed";
import { SeverityBadge, StateBadge } from "../components/Badges";
import { paletteColor } from "../components/colors";
import "./AlertFeed.css";

type Filter = "mine" | "all";
const FILTER_KEY = "alerting.feedFilter";
const FLASH_MS = 3_000;

// Wybór zapamiętany w przeglądarce; `null` = jeszcze nie wybierano.
function readFilter(): Filter | null {
  try {
    const v = localStorage.getItem(FILTER_KEY);
    return v === "all" || v === "mine" ? v : null;
  } catch {
    return null;
  }
}

function ago(iso: string, now: number): string {
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (s < 60) return `${s} s temu`;
  if (s < 3600) return `${Math.floor(s / 60)} min temu`;
  if (s < 86400) return `${Math.floor(s / 3600)} h temu`;
  return new Date(iso).toLocaleString();
}

// Odświeżanie „x s temu” bez zapytań do API.
function useNow(intervalMs: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

export default function AlertFeed() {
  const personId = usePersonId();
  const myTeams = useTeamsOf(personId);
  const { alerts, touched, loading, error } = useAlertFeed();
  const status = useStreamStatus();
  const navigate = useNavigate();
  const now = useNow(5_000);
  const [chosen, setFilterState] = useState<Filter | null>(readFilter);
  // Bez własnego wyboru: „mój zespół” tylko dla osoby, która ma zespół – inaczej feed byłby pusty.
  const filter: Filter = chosen ?? (myTeams?.length ? "mine" : "all");

  const setFilter = (f: Filter) => {
    setFilterState(f);
    try {
      localStorage.setItem(FILTER_KEY, f);
    } catch {
      // bez localStorage filtr żyje do przeładowania
    }
  };

  const visible =
    filter === "all" || !myTeams ? alerts : alerts.filter((a) => a.teams.some((t) => myTeams.includes(t)));
  const lastChange = touched.size ? Math.max(...touched.values()) : null;

  return (
    <section className="feed">
      <header className="feed-header">
        <div>
          <h1>Alerty</h1>
          <p className="muted feed-live" aria-live="polite">
            {status === "live" ? "Na żywo" : status === "unavailable" ? "Bez aktualizacji na żywo (mock)" : "Łączenie…"}
            {lastChange && ` · ostatnia zmiana ${new Date(lastChange).toLocaleTimeString()}`}
          </p>
        </div>
        <div className="segmented" role="group" aria-label="Filtr alertów">
          <button aria-pressed={filter === "mine"} onClick={() => setFilter("mine")}>
            Mój zespół{myTeams?.length ? ` (${myTeams.join(", ")})` : ""}
          </button>
          <button aria-pressed={filter === "all"} onClick={() => setFilter("all")}>
            Wszystkie
          </button>
        </div>
      </header>

      {error && <p className="error-box">{error}</p>}
      {filter === "mine" && myTeams && myTeams.length === 0 && (
        <p className="feed-hint">
          {personId ? `„${personId}” nie należy do żadnego zespołu.` : "Nie wybrano tożsamości."}{" "}
          <button onClick={() => setFilter("all")}>Pokaż wszystkie</button>
        </p>
      )}

      <div className="feed-table-wrap">
        <table className="feed-table">
          <thead>
            <tr>
              <th scope="col">Severity</th>
              <th scope="col">Stan</th>
              <th scope="col">Alert</th>
              <th scope="col">Zespoły</th>
              <th scope="col" className="num">Licznik</th>
              <th scope="col">Ostatnio</th>
            </tr>
          </thead>
          <tbody>
            {loading && (
              <tr>
                <td colSpan={6} className="muted">Ładowanie…</td>
              </tr>
            )}
            {!loading && !visible.length && (
              <tr>
                <td colSpan={6} className="muted">Brak alertów.</td>
              </tr>
            )}
            {visible.map((a) => {
              const t = touched.get(a.id);
              const flash = t !== undefined && Date.now() - t < FLASH_MS;
              const color = paletteColor(a.color);
              return (
                <tr
                  // nowy klucz po zmianie na żywo restartuje animację wyróżnienia
                  key={flash ? `${a.id}:${t}` : a.id}
                  className={flash ? "flash" : undefined}
                  style={color ? { boxShadow: `inset 4px 0 0 ${color.fg}` } : undefined}
                  onClick={() => navigate(`/alerts/${a.id}`)}
                >
                  <td><SeverityBadge severity={a.severity} /></td>
                  <td><StateBadge state={a.state} /></td>
                  <td className="feed-title">
                    <Link to={`/alerts/${a.id}`} onClick={(e) => e.stopPropagation()}>{a.title}</Link>
                    <span className="muted">{a.schema}{a.episode > 1 ? ` · epizod ${a.episode}` : ""}</span>
                  </td>
                  <td>{a.teams.length ? a.teams.join(", ") : <span className="muted">—</span>}</td>
                  <td className="num">×{a.count}</td>
                  <td title={new Date(a.last_seen).toLocaleString()}>{ago(a.last_seen, now)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}
