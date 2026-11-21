import type { ThemeAppearance } from "@atlaskit/lozenge";
import {
  ArchiveX,
  ArrowRightLeft,
  Ban,
  BellRing,
  BookOpen,
  Bot,
  CheckCheck,
  Circle,
  CircleCheck,
  CircleDashed,
  CircleExclamationPoint,
  CirclePause,
  CirclePlay,
  CircleQuestionMark,
  CircleX,
  Clock,
  Eye,
  Hand,
  Info,
  LoaderCircle,
  Lock,
  Mail,
  MessageSquare,
  Monitor,
  Phone,
  Radio,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  ShieldExclamationPoint,
  ShieldQuestionMark,
  Siren,
  SkipForward,
  TimerOff,
  TriangleExclamationPoint,
  Unplug,
  UserRound,
  Wifi,
  WifiOff,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { Schemas } from "../api/client";
import type { StreamStatus } from "../api/stream";

export type StatusDef = { label: string; appearance: ThemeAppearance; isBold?: boolean; icon: LucideIcon };

type Table<V extends string> = Record<V, StatusDef>;
const s = (label: string, appearance: ThemeAppearance, icon: LucideIcon, isBold?: boolean): StatusDef =>
  isBold ? { label, appearance, icon, isBold } : { label, appearance, icon };

const alertState: Table<Schemas["AlertState"]> = {
  open: s("open", "moved", BellRing),
  reopened: s("reopened", "moved", RotateCcw, true),
  acked: s("acknowledged", "inprogress", Eye),
  resolved: s("resolved", "success", CircleCheck),
};

const severity: Table<Schemas["Severity"]> = {
  critical: s("critical", "removed", Siren, true),
  error: s("error", "moved", TriangleExclamationPoint, true),
  warning: s("warning", "moved", CircleExclamationPoint),
  info: s("info", "default", Info),
};

const delivery: Table<Schemas["DeliveryStatus"]> = {
  pending: s("pending", "default", Clock),
  retrying: s("retrying", "moved", RefreshCw),
  delivered: s("delivered", "success", CheckCheck),
  fallback: s("fallback", "inprogress", ArrowRightLeft),
  failed: s("failed", "removed", CircleX),
  dlq: s("DLQ", "removed", ArchiveX, true),
};

const step: Table<Schemas["StepStatus"]> = {
  pending: s("pending", "default", Circle),
  running: s("running", "inprogress", LoaderCircle),
  succeeded: s("done", "success", CircleCheck),
  failed: s("failed", "removed", CircleX),
  waiting_approval: s("waiting approval", "moved", Hand, true),
  waiting_manual: s("manual step", "moved", UserRound),
  skipped: s("skipped", "default", SkipForward),
  rejected: s("rejected", "removed", Ban),
};

const risk: Table<Schemas["ActionDef"]["risk"]> = {
  high: s("high risk", "moved", ShieldExclamationPoint, true),
  low: s("low risk", "default", ShieldCheck),
};

const approval: Table<Exclude<NonNullable<Schemas["StepDef"]["approval"]>, "none">> = {
  required: s("requires approval", "moved", ShieldQuestionMark),
};

const approvalDeadline: Table<"overdue"> = {
  overdue: s("overdue", "removed", TimerOff, true),
};

const channelAvailability: Table<`${Schemas["MockChannelState"]["available"]}`> = {
  true: s("enabled", "success", Wifi),
  false: s("outage", "removed", WifiOff, true),
};

const channel: Table<Schemas["Channel"]> = {
  ui: s("UI", "default", Monitor),
  email: s("Email", "default", Mail),
  sms: s("SMS", "default", MessageSquare),
  phone: s("Phone", "default", Phone),
};

const rule: Table<`${boolean}`> = {
  true: s("enabled", "success", CirclePlay),
  false: s("disabled", "default", CirclePause),
};

const stream: Table<StreamStatus> = {
  idle: s("disconnected", "default", Unplug),
  connecting: s("connecting…", "inprogress", LoaderCircle),
  live: s("live", "success", Radio),
  reconnecting: s("reconnecting…", "moved", RefreshCw),
  unavailable: s("live unavailable (mock)", "default", WifiOff),
};

const runbookKind: Table<Schemas["RunbookDef"]["kind"]> = {
  automatic: s("automatic", "new", Bot),
  manual: s("manual", "default", BookOpen),
};

const stepAutomation: Table<Schemas["StepDef"]["automation"]> = {
  automated: s("automated", "new", Bot),
  manual: s("manual", "moved", Hand),
};

const actionKind: Table<Schemas["ActionDef"]["kind"]> = {
  builtin: s("built-in", "default", Lock),
  http: s("HTTP", "new", Zap),
};

const schema: Table<"unmapped"> = { unmapped: s("UNMAPPED", "moved", CircleQuestionMark) };
const action: Table<"missing"> = { missing: s("missing in registry", "moved", CircleQuestionMark) };

export const STATUS = {
  alertState,
  severity,
  delivery,
  step,
  risk,
  approval,
  approvalDeadline,
  channelAvailability,
  channel,
  rule,
  stream,
  runbookKind,
  stepAutomation,
  actionKind,
  schema,
  action,
} as const;

export type StatusDomain = keyof typeof STATUS;
export type StatusValue<D extends StatusDomain> = keyof (typeof STATUS)[D] & string;

/** Status wartości; nieznana wartość z backendu → neutralny fallback z surową etykietą. */
export function statusOf<D extends StatusDomain>(domain: D, value: StatusValue<D> | string | boolean): StatusDef {
  const key = String(value);
  const table: Record<string, StatusDef> = STATUS[domain];
  return Object.hasOwn(table, key) ? table[key] : { label: key, appearance: "default", icon: CircleDashed };
}
