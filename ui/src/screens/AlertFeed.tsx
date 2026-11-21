// Maksymalnie uproszczona lista aktywnych alertów: tylko nierozwiązane alerty, szybkie rozwiązanie jednym kliknięciem.
import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { api, asPerson } from "../api/client";
import { send } from "../api/request";
import { usePersonId } from "../api/session";
import { useAlertFeed, useTeamsOf } from "../api/useAlertFeed";
import { SeverityBadge } from "../components/Badges";
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

function readFilter(): Filter | null {
  try {
    const v = localStorage.getItem(FILTER_KEY);
    return v === "all" || v === "mine" ? v : null;
  } catch {
    return null;
  }
}

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
  const { alerts, touched, loading, error, retry, remove } = useAlertFeed();
  const navigate = useNavigate();
  const now = useNow(5_000);
  const [chosen, setFilterState] = useState<Filter | null>(readFilter);
  const [resolvingId, setResolvingId] = useState<string | null>(null);
  const filter: Filter = chosen ?? (myTeams?.length ? "mine" : "all");

  const setFilter = (f: Filter) => {
    setFilterState(f);
    try {
      localStorage.setItem(FILTER_KEY, f);
    } catch {
      // bez localStorage filtr żyje do przeładowania
    }
  };

  async function resolveAlert(e: React.MouseEvent, alertId: string) {
    e.stopPropagation();
    if (!personId) return;
    setResolvingId(alertId);
    const err = await send(() =>
      api.POST("/alerts/{alert_id}/resolve", {
        params: { path: { alert_id: alertId }, header: asPerson() },
      }),
    );
    setResolvingId(null);
    if (!err) remove(alertId);
  }

  const visible =
    filter === "all" || !myTeams ? alerts : alerts.filter((a) => a.teams.some((t) => myTeams.includes(t)));

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
      key: flash ? `${a.id}:${t}` : a.id,
      className: [flash && "flash", i === current && "kb-current"].filter(Boolean).join(" ") || undefined,
      style: color ? { boxShadow: `inset 4px 0 0 ${color.fg}` } : undefined,
      onClick: () => navigate(`/alerts/${a.id}`),
      cells: [
        { key: "severity", content: <SeverityBadge severity={a.severity} /> },
        {
          key: "title",
          content: (
            <span className="feed-title">
              <Link to={`/alerts/${a.id}`} onClick={(e) => e.stopPropagation()}>
                {a.title}
              </Link>
            </span>
          ),
        },
        { key: "last", content: <span title={new Date(a.last_seen).toLocaleString()}>{ago(a.last_seen, now)}</span> },
        {
          key: "actions",
          content: (
            <div onClick={(e) => e.stopPropagation()}>
              <Button
                spacing="compact"
                isLoading={resolvingId === a.id}
                isDisabled={!personId || resolvingId !== null}
                onClick={(e) => void resolveAlert(e, a.id)}
              >
                Rozwiąż
              </Button>
            </div>
          ),
        },
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
          emptyView={<EmptyList header="Brak aktywnych alertów" description="Wszystkie alerty są rozwiązane." />}
          head={{
            cells: [
              { key: "severity", content: "Priorytet" },
              { key: "title", content: "Alert" },
              { key: "last", content: "Kiedy" },
              { key: "actions", content: "" },
            ],
          }}
          rows={rows}
        />
      )}
    </section>
  );
}
