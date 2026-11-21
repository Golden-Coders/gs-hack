import Lozenge, { type ThemeAppearance } from "@atlaskit/lozenge";
import type { Schemas } from "../api/client";
import { StatusLozenge } from "../ui/primitives";

export type Tone = "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray";

const TONE: Record<Tone, ThemeAppearance> = {
  red: "removed",
  orange: "moved",
  yellow: "moved",
  green: "success",
  blue: "inprogress",
  purple: "new",
  gray: "default",
};

/** Etykieta bez statusu domeny (kategorie, liczniki); statusy idą przez `StatusLozenge`. */
export function Badge({ tone, outline, children }: { tone: Tone; outline?: boolean; children: string }) {
  return (
    <Lozenge appearance={TONE[tone] ?? "default"} isBold={outline}>
      {children}
    </Lozenge>
  );
}

export function SeverityBadge({ severity }: { severity: Schemas["Severity"] }) {
  return <StatusLozenge domain="severity" value={severity} />;
}

export function StateBadge({ state }: { state: Schemas["AlertState"] }) {
  return <StatusLozenge domain="alertState" value={state} />;
}

export function DeliveryStatusBadge({ status }: { status: Schemas["DeliveryStatus"] }) {
  return <StatusLozenge domain="delivery" value={status} />;
}

export function StepStatusBadge({ status }: { status: Schemas["StepStatus"] }) {
  return <StatusLozenge domain="step" value={status} />;
}

/** Ryzyko akcji: wysokie zawsze wymaga zatwierdzenia człowieka — uwaga (pomarańcz, pogrubione), nie porażka. */
export function RiskBadge({ risk }: { risk: "low" | "high" }) {
  return <StatusLozenge domain="risk" value={risk} />;
}
