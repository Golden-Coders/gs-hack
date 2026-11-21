// Feed alertów na żywo (tasks.md 14.1): filtr „mój zespół / wszystkie”, stan, severity, licznik, ostatnie wystąpienie.
import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { usePersonId } from "../api/session";
import { useStreamStatus } from "../api/stream";
import { useAlertFeed, useTeamsOf } from "../api/useAlertFeed";
import { SeverityBadge, StateBadge } from "../components/Badges";
import { paletteColor } from "../components/colors";
import { ago } from "../ui/format";
import Button from "@atlaskit/button/new";
import DynamicTable from "@atlaskit/dynamic-table";
import SectionMessage from "@atlaskit/section-message";
import { useListKeyboard } from "../ui/keyboard";
import { EmptyList, ErrorBox, ListSkeleton, PageHeader, Segmented } from "../ui/primitives";
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
  const { alerts, touched, loading, error, retry } = useAlertFeed();
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

  const { index: current } = useListKeyboard({
    keys: visible.map((a) => a.id),
    enabled: true,
    onOpen: (id) => navigate(`/alerts/${id}`),
  });

  const rows = visible.map((a, i) => {
    const t = touched.get(a.id);
    const flash = t !== undefined && Date.now() - t < FLASH_MS;
    const color = paletteColor(a.color);
    return {
      // nowy klucz po zmianie na żywo restartuje animację wyróżnienia
      key: flash ? `${a.id}:${t}` : a.id,
      className: [flash && "flash", i === current && "kb-current"].filter(Boolean).join(" ") || undefined,
      style: color ? { boxShadow: `inset 4px 0 0 ${color.fg}` } : undefined,
      onClick: () => navigate(`/alerts/${a.id}`),
      cells: [
        { key: "severity", content: <SeverityBadge severity={a.severity} /> },
        { key: "state", content: <StateBadge state={a.state} /> },
        {
          key: "title",
          content: (
            <span className="feed-title">
              <Link to={`/alerts/${a.id}`} onClick={(e) => e.stopPropagation()}>{a.title}</Link>
              <span className="muted">{a.schema}{a.episode > 1 ? ` · epizod ${a.episode}` : ""}</span>
            </span>
          ),
        },
        { key: "teams", content: a.teams.length ? a.teams.join(", ") : <Text color="color.text.subtle">—</Text> },
        { key: "count", content: <span className="num">×{a.count}</span> },
        { key: "last", content: <span title={new Date(a.last_seen).toLocaleString()}>{ago(a.last_seen, now)}</span> },
      ],
    };
  });

  return (
    <section className="feed">
      <PageHeader
        actions={
          <Segmented
            label="Filtr alertów"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "mine", label: `Mój zespół${myTeams?.length ? ` (${myTeams.join(", ")})` : ""}` },
              { value: "all", label: "Wszystkie" },
            ]}
          />
        }
        bottomBar={
          <Text as="p" color="color.text.subtle" aria-live="polite">
            {status === "live" ? "Na żywo" : status === "unavailable" ? "Bez aktualizacji na żywo (mock)" : "Łączenie…"}
            {lastChange && ` · ostatnia zmiana ${new Date(lastChange).toLocaleTimeString()}`}
          </Text>
        }
      >
        Alerty
      </PageHeader>

      {error && <ErrorBox message={error} onRetry={retry} disabled={loading} />}
      {filter === "mine" && myTeams && myTeams.length === 0 && (
        <SectionMessage appearance="information">
          <Text as="p">
            {personId ? `„${personId}” nie należy do żadnego zespołu.` : "Nie wybrano tożsamości."}{" "}
            <Button onClick={() => setFilter("all")}>Pokaż wszystkie</Button>
          </Text>
        </SectionMessage>
      )}

      {loading && alerts.length === 0 ? (
        <ListSkeleton label="Ładowanie alertów…" />
      ) : (
        <DynamicTable
          label="Alerty"
          highlightedRowIndex={current >= 0 ? current : undefined}
          emptyView={<EmptyList header="Brak alertów" description="Nowe alerty pojawią się tu na żywo." />}
          head={{
            cells: [
              { key: "severity", content: "Severity" },
              { key: "state", content: "Stan" },
              { key: "title", content: "Alert" },
              { key: "teams", content: "Zespoły" },
              { key: "count", content: "Licznik" },
              { key: "last", content: "Ostatnio" },
            ],
          }}
          rows={rows}
        />
      )}
    </section>
  );
}
