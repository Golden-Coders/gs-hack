import { describe, expect, it } from "vitest";
import { STATUS, statusOf, type StatusDomain } from "./status";

// Wartości domen wg kontraktu / design.md D6 (jawna lista: wpis usunięty z status.ts zawiedzie test).
const VALUES: Record<StatusDomain, string[]> = {
  alertState: ["open", "reopened", "acked", "resolved"],
  severity: ["critical", "error", "warning", "info"],
  delivery: ["pending", "retrying", "delivered", "fallback", "failed", "dlq"],
  step: ["pending", "running", "succeeded", "failed", "waiting_approval", "waiting_manual", "skipped", "rejected"],
  risk: ["high", "low"],
  approval: ["required"],
  approvalDeadline: ["overdue"],
  channelAvailability: ["true", "false"],
  channel: ["ui", "email", "sms", "phone"],
  rule: ["true", "false"],
  stream: ["idle", "connecting", "live", "reconnecting", "unavailable"],
  runbookKind: ["automatic", "manual"],
  stepAutomation: ["automated", "manual"],
  actionKind: ["builtin", "http"],
  schema: ["unmapped"],
  action: ["missing"],
};

// Jedyne wartości z czerwienią (R w design.md D6): odrzucone / blokujące / porażka / przekroczony termin.
const RED = [
  "severity.critical",
  "delivery.failed",
  "delivery.dlq",
  "step.failed",
  "step.rejected",
  "approvalDeadline.overdue",
  "channelAvailability.false",
];

const domains = Object.keys(VALUES) as StatusDomain[];
const all = domains.flatMap((d) => VALUES[d].map((v) => [d, v] as const));

describe("STATUS", () => {
  it("ma dokładnie domeny i wartości z D6", () => {
    expect(Object.keys(STATUS).sort()).toEqual([...domains].sort());
    for (const d of domains) expect(Object.keys(STATUS[d]).sort(), d).toEqual([...VALUES[d]].sort());
  });

  it.each(all)("%s.%s ma etykietę, appearance i ikonę", (d, v) => {
    const def = statusOf(d, v);
    expect(def.label.length).toBeGreaterThan(0);
    expect(def.appearance).toBeTruthy();
    expect(def.icon).toBeTruthy();
  });

  it("appearance removed tylko dla wartości oznaczonych R", () => {
    const removed = all.filter(([d, v]) => statusOf(d, v).appearance === "removed").map(([d, v]) => `${d}.${v}`);
    expect(removed.sort()).toEqual([...RED].sort());
  });
});

describe("statusOf", () => {
  it("przyjmuje boolean jak string", () => {
    expect(statusOf("rule", true)).toBe(STATUS.rule.true);
    expect(statusOf("channelAvailability", false)).toBe(STATUS.channelAvailability.false);
  });

  it("nieznana wartość daje neutralny fallback z surową etykietą", () => {
    const def = statusOf("alertState", "zombie");
    expect(def.label).toBe("zombie");
    expect(def.appearance).toBe("default");
    expect(def.icon).toBeTruthy();
    expect(def.icon).not.toBe(STATUS.alertState.open.icon);
  });

  it("klucze prototypu nie są wartościami (hasOwn)", () => {
    expect(statusOf("severity", "toString").label).toBe("toString");
    expect(statusOf("severity", "constructor").appearance).toBe("default");
  });
});
