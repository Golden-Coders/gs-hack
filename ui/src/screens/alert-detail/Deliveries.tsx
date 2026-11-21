// Dostarczenia alertu: wszystkie i nieudane (błąd techniczny kanału / DLQ) z „wyślij ponownie”.
import { useState } from "react";
import { api, asPerson, type Schemas } from "../../api/client";
import { usePersonId } from "../../api/session";
import { DeliveryStatusBadge } from "../../components/Badges";
import { Card, ErrorBox, Tabs } from "../../ui/primitives";
import { send } from "../../api/request";
import type { Resource } from "../../api/useResource";
import { formatTime } from "../../ui/format";

const CHANNEL: Record<Schemas["Channel"], string> = { ui: "UI", email: "e-mail", sms: "SMS", phone: "telefon" };
const isFailed = (d: Schemas["Delivery"]) => d.status === "failed" || d.status === "dlq";

// Kontrakt nie ma pola „łańcuch”: łańcuch fallbacku odtwarzamy z dostarczeń tej samej osoby i epizodu
// (każdy kanał to osobny wiersz, uporządkowany po czasie zlecenia).
function chainOf(all: Schemas["Delivery"][], d: Schemas["Delivery"]) {
  return all
    .filter((x) => x.person_id === d.person_id && x.episode === d.episode)
    .sort((a, b) => a.received_at.localeCompare(b.received_at));
}

function FallbackInfo({ all, d }: { all: Schemas["Delivery"][]; d: Schemas["Delivery"] }) {
  const chain = chainOf(all, d);
  if (chain.length < 2) return null;
  const i = chain.findIndex((x) => x.id === d.id);
  const next = chain[i + 1];
  const prev = chain[i - 1];
  const label =
    d.status === "fallback" && next
      ? `fallback z ${CHANNEL[d.channel]} na ${CHANNEL[next.channel]}`
      : prev?.status === "fallback"
        ? `po fallbacku z ${CHANNEL[prev.channel]}`
        : null;
  return (
    <div className="muted">
      {label && <strong>{label}</strong>}{" "}
      <span title="Łańcuch kanałów dla tej osoby w tym epizodzie">
        {chain.map((x, k) => (
          <span key={x.id}>
            {k > 0 && " → "}
            {x.id === d.id ? <strong>[{CHANNEL[x.channel]}]</strong> : CHANNEL[x.channel]}
          </span>
        ))}
      </span>
    </div>
  );
}

export default function Deliveries({
  resource,
  onChanged,
}: {
  resource: Resource<Schemas["Delivery"][]>;
  onChanged: () => void;
}) {
  const personId = usePersonId();
  const [tab, setTab] = useState<"all" | "failed">("all");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const all = resource.data ?? [];
  const failed = all.filter(isFailed);
  const rows = tab === "failed" ? failed : all;

  async function resend(id: string) {
    setBusyId(id);
    setError(null);
    const err = await send(() =>
      api.POST("/deliveries/{delivery_id}/resend", {
        params: { path: { delivery_id: id }, header: asPerson() },
      }),
    );
    setBusyId(null);
    if (err) setError(`Nie udało się wysłać ponownie: ${err}`);
    else onChanged();
  }

  return (
    <Card title="Dostarczenia" loading={resource.loading} error={resource.error} onRetry={resource.reload}>
      <Tabs
        label="Filtr dostarczeń"
        value={tab}
        onChange={setTab}
        tabs={[
          { value: "all", label: `Wszystkie (${all.length})` },
          { value: "failed", label: `Nieudane (${failed.length})` },
        ]}
      />
      {error && <ErrorBox message={error} />}
      {!resource.loading && !resource.error && rows.length === 0 && (
        <p className="muted">{tab === "failed" ? "Brak nieudanych dostarczeń." : "Brak dostarczeń."}</p>
      )}
      {rows.length > 0 && (
        <div className="table-wrap" role="tabpanel">
          <table className="data-table">
            <thead>
              <tr>
                <th>Osoba</th>
                <th>Kanał</th>
                <th>Status</th>
                <th className="num">Próby</th>
                <th>Epizod</th>
                <th>Zlecono</th>
                <th>Dostarczono</th>
                <th>Błąd</th>
                <th>
                  <span className="sr-only">Akcje</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((d) => (
                <tr key={d.id}>
                  <td>{d.person_id}</td>
                  <td>
                    {CHANNEL[d.channel] ?? d.channel}
                    <FallbackInfo all={all} d={d} />
                  </td>
                  <td>
                    <DeliveryStatusBadge status={d.status} />
                  </td>
                  <td className="num">{d.attempts}</td>
                  <td>#{d.episode}</td>
                  <td className="nowrap">{formatTime(d.received_at)}</td>
                  <td className="nowrap">{formatTime(d.delivered_at)}</td>
                  <td className={d.last_error ? "ad-error-text" : undefined}>{d.last_error ?? "—"}</td>
                  <td>
                    {isFailed(d) && (
                      <button
                        type="button"
                        onClick={() => resend(d.id)}
                        disabled={busyId !== null || !personId}
                        title={personId ? undefined : "Wybierz osobę w polu „Działam jako”"}
                      >
                        {busyId === d.id ? "Wysyłanie…" : "Wyślij ponownie"}
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}
