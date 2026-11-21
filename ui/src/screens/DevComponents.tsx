// Przegląd wspólnych komponentów (tasks.md 13.3) na danych z API – na mocku to przykłady z fixtures OpenAPI.
// Dostępne pod /dev/components, poza menu.
import { useEffect, useState } from "react";

import { api, type Schemas } from "../api/client";
import { DeliveryStatusBadge, RiskBadge, SeverityBadge, StateBadge, StepStatusBadge } from "../components/Badges";
import { Card, ConfirmInline, CopyButton, Empty, ErrorBox, Field, Loading, Segmented, Tabs } from "../ui/primitives";
import Markdown from "../components/Markdown";
import Presentation from "../components/Presentation";

const SEVERITIES: Schemas["Severity"][] = ["critical", "error", "warning", "info"];
const STATES: Schemas["AlertState"][] = ["open", "reopened", "acked", "resolved"];
const DELIVERY: Schemas["DeliveryStatus"][] = ["pending", "retrying", "delivered", "fallback", "failed", "dlq"];
const STEPS: Schemas["StepStatus"][] = [
  "pending", "running", "succeeded", "failed", "waiting_approval", "waiting_manual", "skipped", "rejected",
];

// Markdown z konfiguracji jest niezaufany: HTML i linki javascript: nie mogą się wykonać.
const UNSAFE_MD = "**HTML i skrypty są wyłączone:** <img src=x onerror=alert(1)> [link](javascript:alert(1))";

export default function DevComponents() {
  const [alerts, setAlerts] = useState<Schemas["AlertDetail"][]>([]);
  const [runbooks, setRunbooks] = useState<Schemas["RunbookDef"][]>([]);

  useEffect(() => {
    api.GET("/alerts").then(async ({ data }) => {
      const details = await Promise.all(
        (data ?? []).map((a) => api.GET("/alerts/{alert_id}", { params: { path: { alert_id: a.id } } })),
      );
      setAlerts(details.flatMap((d) => (d.data ? [d.data] : [])));
    });
    api.GET("/runbooks").then(({ data }) => setRunbooks(data ?? []));
  }, []);

  return (
    <section className="dev-components">
      <h1>Komponenty</h1>

      <h3>Severity</h3>
      <div className="row">{SEVERITIES.map((s) => <SeverityBadge key={s} severity={s} />)}</div>
      <h3>Stan alertu</h3>
      <div className="row">{STATES.map((s) => <StateBadge key={s} state={s} />)}</div>
      <h3>Status dostarczenia</h3>
      <div className="row">{DELIVERY.map((s) => <DeliveryStatusBadge key={s} status={s} />)}</div>
      <h3>Status kroku</h3>
      <div className="row">{STEPS.map((s) => <StepStatusBadge key={s} status={s} />)}</div>

      <h3>Ryzyko akcji</h3>
      <div className="row"><RiskBadge risk="high" /><RiskBadge risk="low" /></div>

      <h3>Prymitywy</h3>
      <Primitives />

      <h3>Prezentacja reguły</h3>
      {alerts.map((a) => <Presentation key={a.id} alert={a} />)}

      <h3>Markdown runbooka</h3>
      {runbooks.filter((r) => r.body).map((r) => <Markdown key={r.id} source={r.body!} />)}
      <Markdown source={UNSAFE_MD} />
    </section>
  );
}

function Primitives() {
  const [seg, setSeg] = useState<"mine" | "all">("mine");
  const [tab, setTab] = useState<"all" | "failed">("all");
  return (
    <Card title="Card z akcjami" actions={<button>Odśwież</button>}>
      <div className="row">
        <Segmented label="Przykład" value={seg} onChange={setSeg} options={[{ value: "mine", label: "Mój zespół" }, { value: "all", label: "Wszystkie" }]} />
        <ConfirmInline label="Usuń" question="Usunąć?" confirmLabel="Tak, usuń" onConfirm={() => {}} />
        <CopyButton text="config/p2/people.yaml" />
      </div>
      <Tabs label="Przykład" value={tab} onChange={setTab} tabs={[{ value: "all", label: "Wszystkie (2)" }, { value: "failed", label: "Nieudane (0)" }]} />
      <Field label="URL" hint="Host musi być na ACTION_HOST_ALLOWLIST" error="Podaj adres URL">
        {(p) => <input {...p} />}
      </Field>
      <Loading />
      <Empty>Brak kroków do zatwierdzenia.</Empty>
      <ErrorBox message="HTTP 404" onRetry={() => {}} />
    </Card>
  );
}
