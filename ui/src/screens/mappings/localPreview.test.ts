import { describe, expect, it } from "vitest";
import alertmanager from "../../../../crates/contracts/fixtures/config/mapping.alertmanager.json";
import trade from "../../../../crates/contracts/fixtures/config/mapping.trade.json";
import workflow from "../../../../crates/contracts/fixtures/config/mapping.workflow.json";
import alertmanagerEvent from "../../../../crates/contracts/fixtures/events/alertmanager_high_error_rate.json";
import tradeEvent from "../../../../crates/contracts/fixtures/events/trade_rejected.json";
import workflowEvent from "../../../../crates/contracts/fixtures/events/workflow_failed.json";
import type { Cond, Mapping } from "./draft";
import { evalCondition, localPreview } from "./localPreview";

const wf = workflow as unknown as Mapping;
const am = alertmanager as unknown as Mapping;
const tr = trade as unknown as Mapping;

const base: Mapping = { schema: "s", fields: {}, title: null, severity: { type: "const", value: "info" }, fingerprint: [], resolved: null };

describe("localPreview na fixtures z kontraktu", () => {
  it("workflow: severity error, tytuł z błędu, fingerprint z workflowName|step|error.code", () => {
    const r = localPreview(wf, workflowEvent.payload);
    expect(r.severity).toBe("error");
    expect(r.title).toBe("Counterparty service did not respond in 5000 ms");
    expect(r.fingerprint).toBe("workflow.v1|order-settlement|validate-counterparty|TIMEOUT");
    expect(r.resolved).toBe(false);
    expect(r.origin).toBe("local");
    expect(r.fields).toMatchObject({ workflowName: "order-settlement", step: "validate-counterparty", environment: "prod" });
  });

  it("alertmanager firing: critical, nie resolved, tytuł z annotations.summary", () => {
    const r = localPreview(am, alertmanagerEvent.payload);
    expect(r.severity).toBe("critical");
    expect(r.resolved).toBe(false);
    expect(r.title).toBe("Error rate 12% over the last 5 minutes");
    expect(r.fingerprint).toBe("alertmanager|HighErrorRate|payments-api|eu-west");
  });

  it("alertmanager status=resolved: resolved true, ten sam fingerprint co firing, nadal critical", () => {
    const firing = localPreview(am, alertmanagerEvent.payload);
    const resolved = localPreview(am, { ...alertmanagerEvent.payload, status: "resolved" });
    expect(resolved.resolved).toBe(true);
    expect(resolved.severity).toBe("critical");
    expect(resolved.fingerprint).toBe(firing.fingerprint);
  });

  it("trade: notional 25M -> critical, fingerprint z tradeId, tytuł domyślnie = schemat", () => {
    const r = localPreview(tr, tradeEvent.payload);
    expect(r.severity).toBe("critical");
    expect(r.fingerprint).toBe("trade.rejected.v2|T-998877");
    expect(r.title).toBe("trade.rejected.v2");
    expect(r.fields).toEqual({ desk: "rates", tradeId: "T-998877" });
  });

  it("trade: notional poniżej progu -> warning (severity domyślne), dokładnie 10M też", () => {
    const withNotional = (n: number) => ({ ...tradeEvent.payload, payload: { ...tradeEvent.payload.payload, notional: n } });
    expect(localPreview(tr, withNotional(9_999_999)).severity).toBe("warning");
    expect(localPreview(tr, withNotional(10_000_000)).severity).toBe("warning");
    expect(localPreview(tr, withNotional(10_000_001)).severity).toBe("critical");
  });
});

describe("localPreview: przypadki brzegowe", () => {
  it("severity path: nieznana wartość -> default; wielkość liter w mapie i skali ignorowana", () => {
    const m: Mapping = { ...base, severity: { type: "path", path: "$.sev", map: { ERR: "error" }, default: "info" } };
    expect(localPreview(m, { sev: "err" }).severity).toBe("error");
    expect(localPreview(m, { sev: "CRITICAL" }).severity).toBe("critical");
    expect(localPreview(m, { sev: "WARN" }).severity).toBe("warning");
    expect(localPreview(m, { sev: "bogus" }).severity).toBe("info");
    expect(localPreview(m, {}).severity).toBe("info");
  });

  it("brak fingerprintu w mappingu -> skrót całego payloadu (jak w specyfikacji)", () => {
    expect(localPreview(base, { a: 1 }).fingerprint).toBe("s|(skrót całego payloadu)");
  });

  it("brakujące pole w fingerprint daje pusty segment, bez wyjątku", () => {
    const m: Mapping = { ...base, fingerprint: ["$.a", "$.b"] };
    expect(localPreview(m, { a: "x" }).fingerprint).toBe("s|x|");
  });

  it("tytuł jako szablon {{raw.…}} i {{pole}}", () => {
    const m: Mapping = { ...base, fields: { svc: "$.svc" }, title: "Awaria {{raw.svc}} w {{ svc }}" };
    expect(localPreview(m, { svc: "api" }).title).toBe("Awaria api w api");
  });

  it("brak wartości pola -> null w podglądzie pól", () => {
    const m: Mapping = { ...base, fields: { a: "$.nope" } };
    expect(localPreview(m, {}).fields).toEqual({ a: null });
  });

  it("severity rules: pierwsza spełniona reguła wygrywa", () => {
    const m: Mapping = {
      ...base,
      severity: {
        type: "rules",
        default: "info",
        rules: [
          { when: { field: "raw.n", op: "gt", value: 1 }, value: "error" },
          { when: { field: "raw.n", op: "gt", value: 0 }, value: "critical" },
        ],
      },
    };
    expect(localPreview(m, { n: 5 }).severity).toBe("error");
    expect(localPreview(m, { n: 0 }).severity).toBe("info");
  });
});

describe("evalCondition", () => {
  const raw = { type: "WORKFLOW_FAILED", n: 7, tags: ["a", "b"], env: "prod", nothing: null };
  const L = (field: string, op: Cond["op"], value?: unknown): Cond => ({ field, op, value });

  it("eq/ne porównują jako tekst (liczba vs napis)", () => {
    expect(evalCondition(L("raw.n", "eq", "7"), raw)).toBe(true);
    expect(evalCondition(L("raw.n", "eq", 7), raw)).toBe(true);
    expect(evalCondition(L("raw.n", "ne", 8), raw)).toBe(true);
    expect(evalCondition(L("raw.missing", "eq", "x"), raw)).toBe(false);
    expect(evalCondition(L("raw.missing", "ne", "x"), raw)).toBe(true);
  });

  it("porównania liczbowe", () => {
    expect(evalCondition(L("raw.n", "gt", 6), raw)).toBe(true);
    expect(evalCondition(L("raw.n", "gt", 7), raw)).toBe(false);
    expect(evalCondition(L("raw.n", "gte", 7), raw)).toBe(true);
    expect(evalCondition(L("raw.n", "lt", 8), raw)).toBe(true);
    expect(evalCondition(L("raw.n", "lte", 6), raw)).toBe(false);
    expect(evalCondition(L("raw.missing", "gt", 1), raw)).toBe(false);
    expect(evalCondition(L("raw.env", "gt", 1), raw)).toBe(false);
  });

  it("exists: brak i null to nieobecność", () => {
    expect(evalCondition(L("raw.env", "exists"), raw)).toBe(true);
    expect(evalCondition(L("raw.nothing", "exists"), raw)).toBe(false);
    expect(evalCondition(L("raw.missing", "exists"), raw)).toBe(false);
  });

  it("contains: podciąg tekstu i element tablicy", () => {
    expect(evalCondition(L("raw.type", "contains", "FAILED"), raw)).toBe(true);
    expect(evalCondition(L("raw.tags", "contains", "b"), raw)).toBe(true);
    expect(evalCondition(L("raw.tags", "contains", "z"), raw)).toBe(false);
  });

  it("regex i glob; niepoprawny regex -> false zamiast wyjątku", () => {
    expect(evalCondition(L("raw.type", "regex", "^WORKFLOW_"), raw)).toBe(true);
    expect(evalCondition(L("raw.type", "regex", "("), raw)).toBe(false);
    expect(evalCondition(L("raw.type", "glob", "WORKFLOW_*"), raw)).toBe(true);
    expect(evalCondition(L("raw.type", "glob", "WORKFLOW_?AILED"), raw)).toBe(true);
    expect(evalCondition(L("raw.type", "glob", "WORKFLOW"), raw)).toBe(false);
    expect(evalCondition(L("raw.v", "glob", "a.b"), { v: "axb" })).toBe(false);
  });

  it("in: lista wartości, porównanie jako tekst", () => {
    expect(evalCondition(L("raw.n", "in", [1, 7]), raw)).toBe(true);
    expect(evalCondition(L("raw.env", "in", ["dev", "stg"]), raw)).toBe(false);
    expect(evalCondition(L("raw.env", "in", "prod"), raw)).toBe(false);
  });

  it("all / any / not oraz zagnieżdżenia", () => {
    const a = L("raw.env", "eq", "prod");
    const b = L("raw.n", "gt", 100);
    expect(evalCondition({ all: [a, b] }, raw)).toBe(false);
    expect(evalCondition({ any: [a, b] }, raw)).toBe(true);
    expect(evalCondition({ not: b }, raw)).toBe(true);
    expect(evalCondition({ all: [a, { any: [b, { not: b }] }] }, raw)).toBe(true);
  });

  it("puste all jest spełnione, puste any nie (zgodnie z kreatorem)", () => {
    expect(evalCondition({ all: [] }, raw)).toBe(true);
    expect(evalCondition({ any: [] }, raw)).toBe(false);
  });

  it("pole kanoniczne (bez raw.) w podglądzie mappingu nie jest dostępne -> nie spełnia eq", () => {
    expect(evalCondition(L("severity", "eq", "error"), raw)).toBe(false);
  });
});
