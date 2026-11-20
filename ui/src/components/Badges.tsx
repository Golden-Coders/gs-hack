// Badge severity, stanu alertu oraz statusów dostarczeń i kroków runbooka.
import type { Schemas } from "../api/client";

type Tone = "red" | "orange" | "yellow" | "green" | "blue" | "gray";

function Badge({ tone, children }: { tone: Tone; children: string }) {
  return <span className={`badge badge-${tone}`}>{children}</span>;
}

const SEVERITY: Record<Schemas["Severity"], Tone> = {
  critical: "red",
  error: "orange",
  warning: "yellow",
  info: "blue",
};

export function SeverityBadge({ severity }: { severity: Schemas["Severity"] }) {
  return <Badge tone={SEVERITY[severity] ?? "gray"}>{severity}</Badge>;
}

const STATE: Record<Schemas["AlertState"], [Tone, string]> = {
  open: ["red", "otwarty"],
  reopened: ["orange", "ponownie otwarty"],
  acked: ["yellow", "potwierdzony"],
  resolved: ["green", "rozwiązany"],
};

export function StateBadge({ state }: { state: Schemas["AlertState"] }) {
  const [tone, label] = STATE[state] ?? ["gray", state];
  return <Badge tone={tone}>{label}</Badge>;
}

const DELIVERY: Record<Schemas["DeliveryStatus"], [Tone, string]> = {
  pending: ["gray", "oczekuje"],
  retrying: ["yellow", "ponawianie"],
  delivered: ["green", "dostarczono"],
  fallback: ["orange", "kanał zapasowy"],
  failed: ["red", "błąd"],
  dlq: ["red", "DLQ"],
};

export function DeliveryStatusBadge({ status }: { status: Schemas["DeliveryStatus"] }) {
  const [tone, label] = DELIVERY[status] ?? ["gray", status];
  return <Badge tone={tone}>{label}</Badge>;
}

const STEP: Record<Schemas["StepStatus"], [Tone, string]> = {
  pending: ["gray", "oczekuje"],
  running: ["blue", "w toku"],
  succeeded: ["green", "zrobione"],
  failed: ["red", "błąd"],
  waiting_approval: ["orange", "czeka na zatwierdzenie"],
  waiting_manual: ["yellow", "krok ręczny"],
  skipped: ["gray", "pominięty"],
  rejected: ["red", "odrzucony"],
};

export function StepStatusBadge({ status }: { status: Schemas["StepStatus"] }) {
  const [tone, label] = STEP[status] ?? ["gray", status];
  return <Badge tone={tone}>{label}</Badge>;
}
