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
  open: s("otwarty", "moved", BellRing),
  reopened: s("ponownie otwarty", "moved", RotateCcw, true),
  acked: s("potwierdzony", "inprogress", Eye),
  resolved: s("rozwiązany", "success", CircleCheck),
};

const severity: Table<Schemas["Severity"]> = {
  critical: s("krytyczny", "removed", Siren, true),
  error: s("błąd", "moved", TriangleExclamationPoint, true),
  warning: s("ostrzeżenie", "moved", CircleExclamationPoint),
  info: s("informacja", "default", Info),
};

const delivery: Table<Schemas["DeliveryStatus"]> = {
  pending: s("oczekuje", "default", Clock),
  retrying: s("ponawianie", "moved", RefreshCw),
  delivered: s("dostarczono", "success", CheckCheck),
  fallback: s("kanał zapasowy", "inprogress", ArrowRightLeft),
  failed: s("błąd", "removed", CircleX),
  dlq: s("DLQ", "removed", ArchiveX, true),
};

const step: Table<Schemas["StepStatus"]> = {
  pending: s("oczekuje", "default", Circle),
  running: s("w toku", "inprogress", LoaderCircle),
  succeeded: s("zrobione", "success", CircleCheck),
  failed: s("błąd", "removed", CircleX),
  waiting_approval: s("czeka na zatwierdzenie", "moved", Hand, true),
  waiting_manual: s("krok ręczny", "moved", UserRound),
  skipped: s("pominięty", "default", SkipForward),
  rejected: s("odrzucony", "removed", Ban),
};

const risk: Table<Schemas["ActionDef"]["risk"]> = {
  high: s("ryzyko: wysokie", "moved", ShieldExclamationPoint, true),
  low: s("ryzyko: niskie", "default", ShieldCheck),
};

const approval: Table<Exclude<NonNullable<Schemas["StepDef"]["approval"]>, "none">> = {
  required: s("wymaga zatwierdzenia", "moved", ShieldQuestionMark),
};

const approvalDeadline: Table<"overdue"> = {
  overdue: s("przekroczony termin", "removed", TimerOff, true),
};

const channelAvailability: Table<`${Schemas["MockChannelState"]["available"]}`> = {
  true: s("włączony", "success", Wifi),
  false: s("awaria", "removed", WifiOff, true),
};

const channel: Table<Schemas["Channel"]> = {
  ui: s("UI", "default", Monitor),
  email: s("e-mail", "default", Mail),
  sms: s("SMS", "default", MessageSquare),
  phone: s("telefon", "default", Phone),
};

const rule: Table<`${boolean}`> = {
  true: s("włączona", "success", CirclePlay),
  false: s("wyłączona", "default", CirclePause),
};

const stream: Table<StreamStatus> = {
  idle: s("rozłączono", "default", Unplug),
  connecting: s("łączenie…", "inprogress", LoaderCircle),
  live: s("na żywo", "success", Radio),
  reconnecting: s("ponowne łączenie…", "moved", RefreshCw),
  unavailable: s("na żywo niedostępne (mock)", "default", WifiOff),
};

const runbookKind: Table<Schemas["RunbookDef"]["kind"]> = {
  automatic: s("automatyczny", "new", Bot),
  manual: s("ręczny", "default", BookOpen),
};

const stepAutomation: Table<Schemas["StepDef"]["automation"]> = {
  automated: s("automatyczny", "new", Bot),
  manual: s("do wykonania ręcznie", "moved", Hand),
};

const actionKind: Table<Schemas["ActionDef"]["kind"]> = {
  builtin: s("wbudowana", "default", Lock),
  http: s("HTTP", "new", Zap),
};

const schema: Table<"unmapped"> = { unmapped: s("UNMAPPED", "moved", CircleQuestionMark) };
const action: Table<"missing"> = { missing: s("brak w rejestrze", "moved", CircleQuestionMark) };

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
