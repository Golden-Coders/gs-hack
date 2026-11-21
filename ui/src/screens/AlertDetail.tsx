// Maksymalnie uproszczony widok szczegółów alertu:
// - kafel alertu (tytuł, opis, najważniejsze pola) + przycisk Rozwiąż alert
// - po kliknięciu Rozwiąż (lub gdy alert zostanie rozwiązany) natychmiast wraca do listy i znika z panelu
// - sekcja Runbooki (z bezpośrednim przejściem do odpowiedniego kafelka w zatwierdzeniach)
import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import Button from "@atlaskit/button/new";
import { ErrorBox, Loading } from "../ui/primitives";
import { useResultFlag } from "./alert-detail/flags";
import Presentation from "../components/Presentation";
import Runbooks from "./alert-detail/Runbooks";
import { send } from "../api/request";
import { useResource } from "../api/useResource";
import "./AlertDetail.css";

const FOLLOW_UP_MS = 1500;

export default function AlertDetail() {
  const { alertId = "" } = useParams();
  const navigate = useNavigate();
  const path = { alert_id: alertId };

  const alert = useResource(() => api.GET("/alerts/{alert_id}", { params: { path } }), alertId);
  const runbooks = useResource(() => api.GET("/alerts/{alert_id}/runbooks", { params: { path } }), alertId);

  useStream((msg) => {
    if (msg.type === "alert_changed") {
      if (msg.data.alert_id !== alertId) return;
      if (msg.data.state === "resolved") {
        navigate("/", { replace: true });
        return;
      }
      alert.reload();
      runbooks.reload();
    } else if (msg.type === "ui_event") {
      if (msg.data.alert_id !== alertId) return;
      if (msg.data.kind === "step_status") runbooks.reload();
    } else if (msg.alerts.some((a) => a.id === alertId)) {
      alert.reload();
      runbooks.reload();
    }
  });

  usePollWhenQuiet(() => [alert, runbooks].forEach((r) => r.refresh()));

  const followUp = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => void (followUp.current && clearTimeout(followUp.current)), []);
  const refreshAfterCommand = (reload: () => void) => {
    reload();
    if (followUp.current) clearTimeout(followUp.current);
    followUp.current = setTimeout(reload, FOLLOW_UP_MS);
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
          <AlertActions alert={a} onResolved={() => navigate("/", { replace: true })} />
          <Runbooks resource={runbooks} onChanged={() => refreshAfterCommand(runbooks.reload)} />
        </>
      )}
    </section>
  );
}

function AlertActions({
  alert: a,
  onResolved,
}: {
  alert: Schemas["AlertDetail"];
  onResolved: () => void;
}) {
  const personId = usePersonId();
  const [busy, setBusy] = useState(false);
  const flag = useResultFlag();

  async function resolve() {
    setBusy(true);
    const params = { path: { alert_id: a.id }, header: asPerson() };
    const err = await send(() => api.POST("/alerts/{alert_id}/resolve", { params }));
    setBusy(false);
    flag(err, "Rozwiązano alert", "Nie udało się rozwiązać alertu");
    if (!err) onResolved();
  }

  const resolved = a.state === "resolved";
  const noPerson = !personId;
  return (
    <div className="ad-meta">
      <div className="ad-toolbar">
        <Button
          appearance="primary"
          isLoading={busy}
          isDisabled={busy || noPerson || resolved}
          title={noPerson ? "Wybierz osobę w polu „Działam jako”" : undefined}
          onClick={() => void resolve()}
        >
          Rozwiąż alert
        </Button>
        {noPerson && <Text color="color.text.subtle">Wybierz osobę w polu „Działam jako”, aby działać na alercie.</Text>}
      </div>
    </div>
  );
}
