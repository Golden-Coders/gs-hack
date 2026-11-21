// Szczegóły alertu (tasks.md 14.2): prezentacja wg reguły, epizody, runbooki, dostarczenia, dziennik.
// Każda sekcja ładuje się osobno i odświeża na żywo po zdarzeniach z `/stream` dotyczących tego alertu.
import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import Button from "@atlaskit/button/new";
import { ErrorBox, Loading } from "../ui/primitives";
import { useResultFlag } from "./alert-detail/flags";
import Presentation from "../components/Presentation";
import Audit from "./alert-detail/Audit";
import Deliveries from "./alert-detail/Deliveries";
import Episodes from "./alert-detail/Episodes";
import Runbooks from "./alert-detail/Runbooks";
import { send } from "../api/request";
import { useResource } from "../api/useResource";
import { actorLabel, formatTime } from "../ui/format";
import "./AlertDetail.css";

// Komendy trafiają do Kafki asynchronicznie – po przyjęciu odświeżamy od razu i jeszcze raz chwilę później
// (na mocku nie ma strumienia, który by to zrobił).
const FOLLOW_UP_MS = 1500;

export default function AlertDetail() {
  const { alertId = "" } = useParams();
  const path = { alert_id: alertId };

  const alert = useResource(() => api.GET("/alerts/{alert_id}", { params: { path } }), alertId);
  const episodes = useResource(() => api.GET("/alerts/{alert_id}/episodes", { params: { path } }), alertId);
  const runbooks = useResource(() => api.GET("/alerts/{alert_id}/runbooks", { params: { path } }), alertId);
  const deliveries = useResource(() => api.GET("/alerts/{alert_id}/deliveries", { params: { path } }), alertId);
  const audit = useResource(() => api.GET("/alerts/{alert_id}/audit", { params: { path } }), alertId);

  const reloadAlert = () => {
    alert.reload();
    episodes.reload();
    audit.reload();
  };

  useStream((msg) => {
    if (msg.type === "alert_changed") {
      if (msg.data.alert_id !== alertId) return;
      reloadAlert();
      runbooks.reload();
    } else if (msg.type === "ui_event") {
      if (msg.data.alert_id !== alertId) return;
      switch (msg.data.kind) {
        case "alert_count":
          alert.reload();
          episodes.reload();
          break;
        case "delivery_status":
          deliveries.reload();
          break;
        case "step_status":
          runbooks.reload();
          break;
        case "audit_entry":
          audit.reload();
          break;
      }
    } else if (msg.alerts.some((a) => a.id === alertId)) {
      // Po przerwie w połączeniu odświeżamy wszystko.
      reloadAlert();
      runbooks.reload();
      deliveries.reload();
    }
  });

  // Bez zdarzeń na żywo dociągamy całość w tle (bez wskaźników ładowania).
  usePollWhenQuiet(() => [alert, episodes, runbooks, deliveries, audit].forEach((r) => r.refresh()));

  // Odświeżenie po komendzie użytkownika; kolejne wywołanie zastępuje oczekujące.
  const followUp = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => void (followUp.current && clearTimeout(followUp.current)), []);
  const refreshAfterCommand = (reload: () => void) => {
    reload();
    if (followUp.current) clearTimeout(followUp.current);
    followUp.current = setTimeout(() => {
      reload();
      audit.reload();
    }, FOLLOW_UP_MS);
  };

  const a = alert.data;
  return (
    <section className="alert-detail">
      <div className="ad-back">
        <Link to="/">← Wszystkie alerty</Link>
      </div>

      {alert.error && (
        <ErrorBox message={`Nie udało się pobrać alertu: ${alert.error}`} onRetry={alert.reload} disabled={alert.loading} />
      )}
      {!a && alert.loading && <Loading>Ładowanie alertu…</Loading>}

      {a && (
        <>
          <Presentation alert={a} />
          <AlertMeta alert={a} onChanged={() => refreshAfterCommand(reloadAlert)} />
        </>
      )}

      {a && (
        <>
          <Runbooks resource={runbooks} onChanged={() => refreshAfterCommand(runbooks.reload)} />
          <Deliveries resource={deliveries} onChanged={() => refreshAfterCommand(deliveries.reload)} />
          <Episodes resource={episodes} />
          <Audit resource={audit} />
        </>
      )}
    </section>
  );
}

function AlertMeta({
  alert: a,
  onChanged,
}: {
  alert: Schemas["AlertDetail"];
  onChanged: () => void;
}) {
  const personId = usePersonId();
  const [busy, setBusy] = useState<"ack" | "resolve" | null>(null);
  const flag = useResultFlag();

  async function run(kind: "ack" | "resolve") {
    setBusy(kind);
    const params = { path: { alert_id: a.id }, header: asPerson() };
    const err = await send(() =>
      kind === "ack" ? api.POST("/alerts/{alert_id}/ack", { params }) : api.POST("/alerts/{alert_id}/resolve", { params }),
    );
    setBusy(null);
    const name = kind === "ack" ? "potwierdź" : "rozwiąż";
    flag(err, `Wykonano „${name}”`, `Nie udało się wykonać „${name}”`);
    if (!err) onChanged();
  }

  const resolved = a.state === "resolved";
  const noPerson = !personId;
  return (
    <div className="ad-meta">
      <div className="ad-toolbar">
        <Button
          appearance="primary"
          isLoading={busy === "ack"}
          isDisabled={busy !== null || noPerson || a.state === "acked" || resolved}
          title={noPerson ? "Wybierz osobę w polu „Działam jako”" : undefined}
          onClick={() => run("ack")}
        >
          Potwierdź (ack)
        </Button>
        <Button
          isLoading={busy === "resolve"}
          isDisabled={busy !== null || noPerson || resolved}
          title={noPerson ? "Wybierz osobę w polu „Działam jako”" : undefined}
          onClick={() => run("resolve")}
        >
          Rozwiąż (resolve)
        </Button>
        {noPerson && <Text color="color.text.subtle">Wybierz osobę w polu „Działam jako”, aby działać na alercie.</Text>}
      </div>
      <dl className="ad-facts">
        <div>
          <dt>Licznik zdarzeń</dt>
          <dd>{a.count}</dd>
        </div>
        <div>
          <dt>Epizod</dt>
          <dd>#{a.episode}</dd>
        </div>
        <div>
          <dt>Pierwsze zdarzenie</dt>
          <dd>{formatTime(a.first_seen)}</dd>
        </div>
        <div>
          <dt>Ostatnie zdarzenie</dt>
          <dd>{formatTime(a.last_seen)}</dd>
        </div>
        <div>
          <dt>Schemat</dt>
          <dd>{a.schema}</dd>
        </div>
        <div>
          <dt>Reguły</dt>
          <dd>{a.rule_ids.length ? a.rule_ids.join(", ") : "—"}</dd>
        </div>
        <div>
          <dt>Zespoły</dt>
          <dd>{a.teams.length ? a.teams.join(", ") : "—"}</dd>
        </div>
        <div>
          <dt>Potwierdził</dt>
          <dd>{actorLabel(a.acked_by)}</dd>
        </div>
        <div>
          <dt>Rozwiązał</dt>
          <dd>{actorLabel(a.resolved_by)}</dd>
        </div>
      </dl>
    </div>
  );
}
