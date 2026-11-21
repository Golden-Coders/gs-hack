// Wysłane SMS / telefony z symulowanych kanałów + przełącznik dostępności kanału — tasks.md 14.4.
// Wyłączenie kanału pokazuje awarię: kolejne dostarczenia kończą się błędem, a w szczegółach alertu widać fallback.
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Badge } from "../components/Badges";
import { formatTime } from "../ui/format";
import { Card, Empty, ErrorBox } from "../ui/primitives";
import "./MockOutbox.css";

type ChannelState = Schemas["MockChannelState"];

const CHANNEL_LABEL: Record<Schemas["Channel"], string> = {
  ui: "UI",
  email: "E-mail",
  sms: "SMS",
  phone: "Telefon",
};

function ChannelSwitch({
  state,
  pending,
  onToggle,
}: {
  state: ChannelState;
  pending: boolean;
  onToggle: (c: ChannelState) => void;
}) {
  const name = CHANNEL_LABEL[state.channel] ?? state.channel;
  return (
    <li className={`channel-card ${state.available ? "channel-ok" : "channel-down"}`}>
      <div>
        <strong>{name}</strong>
        <div className="channel-state">
          {state.available ? "Stan: włączony" : "Stan: AWARIA (kanał wyłączony)"}
        </div>
      </div>
      <button
        type="button"
        className={state.available ? "danger" : "primary"}
        disabled={pending}
        onClick={() => onToggle(state)}
      >
        {pending
          ? "Zapisywanie…"
          : state.available
            ? `Wyłącz ${name} (symuluj awarię)`
            : `Włącz ${name}`}
      </button>
    </li>
  );
}

export default function MockOutbox() {
  const messages = useResource(() => api.GET("/mock/outbox"));
  const channels = useResource(() => api.GET("/mock/channels"));
  const [toggleError, setToggleError] = useState<string | null>(null);
  const [pending, setPending] = useState<Set<string>>(new Set());
  // Stan kanału zapisany przed odświeżeniem listy – pokazujemy go od razu, serwer potwierdzi przy następnym pobraniu.
  const [saved, setSaved] = useState<Record<string, boolean>>({});

  useEffect(() => setSaved({}), [channels.data]);

  const reload = () => {
    messages.reload();
    channels.reload();
  };

  // Na żywo: nowa „wysłana” wiadomość albo zmiana statusu dostarczenia.
  useStream((msg) => {
    if (msg.type !== "ui_event") return;
    if (msg.data.kind === "mock_outbox" || msg.data.kind === "delivery_status") messages.refresh();
  });

  usePollWhenQuiet(() => {
    messages.refresh();
    channels.refresh();
  });

  async function toggle(state: ChannelState) {
    const channel = state.channel;
    setPending((s) => new Set(s).add(channel));
    setToggleError(null);
    const err = await send(() =>
      api.PUT("/mock/channels/{channel}", { params: { path: { channel } }, body: { available: !state.available } }),
    );
    if (err) {
      setToggleError(`Nie udało się zmienić stanu kanału ${CHANNEL_LABEL[channel] ?? channel}.`);
    } else {
      setSaved((s) => ({ ...s, [channel]: !state.available }));
      channels.refresh();
    }
    setPending((s) => {
      const n = new Set(s);
      n.delete(channel);
      return n;
    });
  }

  const rows = messages.data;
  return (
    <section className="mock-outbox">
      <header className="mock-head">
        <h1>Wysłane SMS / telefony</h1>
        <button type="button" onClick={reload}>
          Odśwież
        </button>
      </header>
      <p className="muted">
        Symulowane kanały — nic nie wychodzi na zewnątrz. Wyłącz kanał, aby pokazać awarię: kolejne dostarczenia
        przez ten kanał zakończą się błędem, a w szczegółach alertu zobaczysz kanał zapasowy.
      </p>

      <Card title="Dostępność kanałów" loading={channels.loading} error={channels.error} onRetry={channels.reload}>
        {toggleError && <ErrorBox message={toggleError} />}
        {channels.data && (
          <ul className="channel-list" aria-label="Dostępność symulowanych kanałów">
            {channels.data.map((c) => (
              <ChannelSwitch
                key={c.channel}
                state={{ ...c, available: saved[c.channel] ?? c.available }}
                pending={pending.has(c.channel)}
                onToggle={toggle}
              />
            ))}
          </ul>
        )}
      </Card>

      <Card title="Wysłane wiadomości" loading={messages.loading} error={messages.error} onRetry={messages.reload}>
        {rows?.length === 0 && <Empty>Brak wysłanych wiadomości</Empty>}
        {rows && rows.length > 0 && (
          <div className="table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Czas</th>
                  <th scope="col">Kanał</th>
                  <th scope="col">Do</th>
                  <th scope="col">Treść</th>
                  <th scope="col">Alert</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((m) => (
                  <tr key={m.id}>
                    <td className="nowrap">{formatTime(m.sent_at)}</td>
                    <td>
                      <Badge tone={m.channel === "phone" ? "blue" : "gray"}>{CHANNEL_LABEL[m.channel] ?? m.channel}</Badge>
                    </td>
                    <td>
                      <div>{m.person_id}</div>
                      <div className="muted outbox-to">{m.to}</div>
                    </td>
                    <td>
                      <strong>{m.title}</strong>
                      <div className="outbox-body">{m.body}</div>
                    </td>
                    <td>
                      {m.alert_id ? <Link to={`/alerts/${m.alert_id}`}>Szczegóły alertu</Link> : <span className="muted">—</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </section>
  );
}
