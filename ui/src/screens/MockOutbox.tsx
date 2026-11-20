// Wysłane SMS / telefony z symulowanych kanałów + przełącznik dostępności kanału — tasks.md 14.4.
// Wyłączenie kanału pokazuje awarię: kolejne dostarczenia kończą się błędem, a w szczegółach alertu widać fallback.
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { useStream } from "../api/stream";
import "./MockOutbox.css";

type Message = Schemas["MockMessage"];
type ChannelState = Schemas["MockChannelState"];

const CHANNEL_LABEL: Record<Schemas["Channel"], string> = {
  ui: "UI",
  email: "E-mail",
  sms: "SMS",
  phone: "Telefon",
};

const dateFmt = new Intl.DateTimeFormat("pl-PL", { dateStyle: "short", timeStyle: "medium" });

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
  const [messages, setMessages] = useState<Message[] | null>(null);
  const [channels, setChannels] = useState<ChannelState[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toggleError, setToggleError] = useState<string | null>(null);
  const [pending, setPending] = useState<Set<string>>(new Set());

  const loadMessages = useCallback(async () => {
    try {
      const { data, error: err } = await api.GET("/mock/outbox");
      if (err || !data) throw new Error("outbox");
      setMessages(data);
      setError(null);
    } catch {
      setError("Nie udało się pobrać wysłanych wiadomości.");
    }
  }, []);

  const loadChannels = useCallback(async () => {
    try {
      const { data, error: err } = await api.GET("/mock/channels");
      if (err || !data) throw new Error("channels");
      setChannels(data);
      setError(null);
    } catch {
      setError("Nie udało się pobrać stanu kanałów.");
    }
  }, []);

  const refresh = useCallback(() => {
    void loadMessages();
    void loadChannels();
  }, [loadMessages, loadChannels]);

  useEffect(refresh, [refresh]);

  // Na żywo: nowa „wysłana” wiadomość albo zmiana statusu dostarczenia.
  useStream((msg) => {
    if (msg.type !== "ui_event") return;
    if (msg.data.kind === "mock_outbox") void loadMessages();
    if (msg.data.kind === "delivery_status") void loadMessages();
  });

  async function toggle(state: ChannelState) {
    const channel = state.channel;
    setPending((s) => new Set(s).add(channel));
    setToggleError(null);
    try {
      const { data, error: err } = await api.PUT("/mock/channels/{channel}", {
        params: { path: { channel } },
        body: { available: !state.available },
      });
      if (err || !data) throw new Error("toggle");
      setChannels((cur) => cur?.map((c) => (c.channel === channel ? { ...c, available: data.available } : c)) ?? cur);
    } catch {
      setToggleError(`Nie udało się zmienić stanu kanału ${CHANNEL_LABEL[channel] ?? channel}.`);
    } finally {
      setPending((s) => {
        const n = new Set(s);
        n.delete(channel);
        return n;
      });
    }
  }

  return (
    <section className="mock-outbox">
      <header className="mock-head">
        <h1>Wysłane SMS / telefony</h1>
        <button type="button" onClick={refresh}>
          Odśwież
        </button>
      </header>
      <p className="muted">
        Symulowane kanały — nic nie wychodzi na zewnątrz. Wyłącz kanał, aby pokazać awarię: kolejne dostarczenia
        przez ten kanał zakończą się błędem, a w szczegółach alertu zobaczysz kanał zapasowy.
      </p>

      {error && (
        <p className="error-box" role="alert">
          {error}
        </p>
      )}

      <h2>Dostępność kanałów</h2>
      {toggleError && (
        <p className="error-box" role="alert">
          {toggleError}
        </p>
      )}
      {channels === null && !error && <p className="muted">Ładowanie…</p>}
      {channels && (
        <ul className="channel-list" aria-label="Dostępność symulowanych kanałów">
          {channels.map((c) => (
            <ChannelSwitch key={c.channel} state={c} pending={pending.has(c.channel)} onToggle={toggle} />
          ))}
        </ul>
      )}

      <h2>Wysłane wiadomości</h2>
      {messages === null && !error && <p className="muted">Ładowanie…</p>}
      {messages?.length === 0 && <p className="muted outbox-empty">Brak wysłanych wiadomości</p>}
      {messages && messages.length > 0 && (
        <div className="outbox-scroll">
          <table className="outbox-table">
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
              {messages.map((m) => (
                <tr key={m.id}>
                  <td className="nowrap">{dateFmt.format(new Date(m.sent_at))}</td>
                  <td>
                    <span className={`badge ${m.channel === "phone" ? "badge-blue" : "badge-gray"}`}>
                      {CHANNEL_LABEL[m.channel] ?? m.channel}
                    </span>
                  </td>
                  <td>
                    <div>{m.person_id}</div>
                    <div className="muted outbox-to">{m.to}</div>
                  </td>
                  <td>
                    <strong>{m.title}</strong>
                    <div className="outbox-body">{m.body}</div>
                  </td>
                  <td>{m.alert_id ? <Link to={`/alerts/${m.alert_id}`}>Szczegóły alertu</Link> : <span className="muted">—</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
