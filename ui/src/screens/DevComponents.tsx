import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { useEffect, useState } from "react";

import { api, type Schemas } from "../api/client";
import { DeliveryStatusBadge, RiskBadge, SeverityBadge, StateBadge, StepStatusBadge } from "../components/Badges";
import { Card, ConfirmInline, CopyButton, Empty, ErrorBox, Field, Loading, Segmented, Tabs, TextInput } from "../ui/primitives";
import Markdown from "../components/Markdown";
import Presentation from "../components/Presentation";

const SEVERITIES: Schemas["Severity"][] = ["critical", "error", "warning", "info"];
const STATES: Schemas["AlertState"][] = ["open", "reopened", "acked", "resolved"];
const DELIVERY: Schemas["DeliveryStatus"][] = ["pending", "retrying", "delivered", "fallback", "failed", "dlq"];
const STEPS: Schemas["StepStatus"][] = [
  "pending", "running", "succeeded", "failed", "waiting_approval", "waiting_manual", "skipped", "rejected",
];

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
      <Heading size="xlarge" as="h1">Komponenty</Heading>

      <Heading size="small" as="h3">Severity</Heading>
      <div className="row">{SEVERITIES.map((s) => <SeverityBadge key={s} severity={s} />)}</div>
      <Heading size="small" as="h3">Stan alertu</Heading>
      <div className="row">{STATES.map((s) => <StateBadge key={s} state={s} />)}</div>
      <Heading size="small" as="h3">Status dostarczenia</Heading>
      <div className="row">{DELIVERY.map((s) => <DeliveryStatusBadge key={s} status={s} />)}</div>
      <Heading size="small" as="h3">Status kroku</Heading>
      <div className="row">{STEPS.map((s) => <StepStatusBadge key={s} status={s} />)}</div>

      <Heading size="small" as="h3">Ryzyko akcji</Heading>
      <div className="row"><RiskBadge risk="high" /><RiskBadge risk="low" /></div>

      <Heading size="small" as="h3">Prymitywy</Heading>
      <Primitives />

      <Heading size="small" as="h3">Prezentacja reguły</Heading>
      {alerts.map((a) => <Presentation key={a.id} alert={a} />)}

      <Heading size="small" as="h3">Markdown runbooka</Heading>
      {runbooks.filter((r) => r.body).map((r) => <Markdown key={r.id} source={r.body!} />)}
      <Markdown source={UNSAFE_MD} />
    </section>
  );
}

function Primitives() {
  const [seg, setSeg] = useState<"mine" | "all">("mine");
  const [tab, setTab] = useState<"all" | "failed">("all");
  return (
    <Card title="Card z akcjami" actions={<Button>Odśwież</Button>}>
      <div className="row">
        <Segmented label="Przykład" value={seg} onChange={setSeg} options={[{ value: "mine", label: "Mój zespół" }, { value: "all", label: "Wszystkie" }]} />
        <ConfirmInline label="Usuń" question="Usunąć?" confirmLabel="Tak, usuń" onConfirm={() => {}} />
        <CopyButton text="config/people.yaml" />
      </div>
      <Tabs label="Przykład" value={tab} onChange={setTab} tabs={[{ value: "all", label: "Wszystkie (2)" }, { value: "failed", label: "Nieudane (0)" }]} />
      <Field label="URL" hint="Host musi być na ACTION_HOST_ALLOWLIST" error="Podaj adres URL">
        {(p) => <TextInput {...p} />}
      </Field>
      <Loading />
      <Empty>Brak kroków do zatwierdzenia.</Empty>
      <ErrorBox message="HTTP 404" onRetry={() => {}} />
    </Card>
  );
}
