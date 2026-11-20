// „Testuj na zdarzeniu” (tasks.md 15.1): wklejone zdarzenie albo ostatnie po event_id -> POST /rules/test.
// Pokazuje wynik każdego warunku z osobna (który liść pasował) i wyrenderowaną prezentację.
import { useState } from "react";
import { api, type Schemas } from "../../api/client";
import { formatValue } from "../../components/Presentation";
import { OP_LABEL, skeletonFromFields, type Rule } from "./model";

type Explanation = Schemas["Explanation"];
type CanonicalEvent = Schemas["CanonicalEvent"];

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const ZERO_UUID = "00000000-0000-0000-0000-000000000000";

/** Pełne zdarzenie kanoniczne zostaje; sam payload trafia do pola `raw`, a brakujące pola dostają wartości domyślne. */
function toCanonicalEvent(v: Record<string, unknown>): CanonicalEvent {
  const base: Record<string, unknown> = "raw" in v ? v : { raw: v };
  const uuid = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : ZERO_UUID;
  return {
    event_id: (base.event_id as string) ?? uuid,
    schema: (base.schema as string) ?? "",
    source: (base.source as string) ?? "test",
    severity: (base.severity as CanonicalEvent["severity"]) ?? "info",
    title: (base.title as string) ?? "",
    fingerprint: (base.fingerprint as string) ?? "test",
    resolved: (base.resolved as boolean) ?? false,
    fields: (base.fields as Record<string, unknown>) ?? {},
    raw: base.raw,
    received_at: (base.received_at as string) ?? new Date().toISOString(),
    mapping_version: (base.mapping_version as number) ?? 0,
  };
}

const NODE_RE = /^(\S+) (eq|ne|in|gt|gte|lt|lte|contains|regex|exists|glob)(?: (.*))?$/;

function nodeLabel(node: string): { kind: "group" | "leaf"; text: string } {
  if (node === "all") return { kind: "group", text: "wszystkie warunki" };
  if (node === "any") return { kind: "group", text: "dowolny warunek" };
  if (node === "not") return { kind: "group", text: "nie" };
  const m = NODE_RE.exec(node);
  if (!m) return { kind: "leaf", text: node };
  return { kind: "leaf", text: `${m[1]} ${OP_LABEL[m[2]] ?? m[2]}${m[3] !== undefined ? ` ${m[3]}` : ""}` };
}

function ResultIcon({ ok }: { ok: boolean }) {
  return ok ? (
    <svg className="rt-icon rt-ok" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path d="M3 8.5l3.2 3L13 4.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  ) : (
    <svg className="rt-icon rt-fail" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path d="M4 4l8 8M12 4l-8 8" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function ExplanationNode({ e }: { e: Explanation }) {
  const label = nodeLabel(e.node);
  return (
    <li>
      <div className={`rt-node ${e.result ? "rt-pass" : "rt-miss"}`}>
        <ResultIcon ok={e.result} />
        <span className={label.kind === "group" ? "rt-group-label" : "mono rt-leaf-label"}>{label.text}</span>
        <span className="rt-verdict">{e.result ? "spełniony" : "niespełniony"}</span>
        {label.kind === "leaf" && e.actual !== undefined && (
          <span className="muted rt-actual">
            w zdarzeniu: <span className="mono">{e.actual === null ? "brak pola" : formatValue(e.actual)}</span>
          </span>
        )}
      </div>
      {e.children.length > 0 && (
        <ul>
          {e.children.map((c, i) => (
            <ExplanationNode key={i} e={c} />
          ))}
        </ul>
      )}
    </li>
  );
}

interface Props {
  /** Aktualna (także niezapisana) reguła; null, gdy w formularzu są błędy blokujące test. */
  rule: Rule | null;
  blockedReason?: string;
  /** Podpowiedziane pola schematu – do wstawienia szkieletu zdarzenia. */
  suggestedFields: string[];
}

export default function RuleTester({ rule, blockedReason, suggestedFields }: Props) {
  const [mode, setMode] = useState<"paste" | "id">("paste");
  const [text, setText] = useState("");
  const [eventId, setEventId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<Schemas["RuleTestResult"] | null>(null);

  async function run() {
    if (!rule) return;
    setError(null);
    let body: { rule: Rule; event?: CanonicalEvent; event_id?: string };
    if (mode === "paste") {
      let parsed: unknown;
      try {
        parsed = JSON.parse(text);
      } catch (e) {
        setError(`To nie jest poprawny JSON: ${e instanceof Error ? e.message : String(e)}`);
        return;
      }
      if (!isObj(parsed)) {
        setError("Zdarzenie musi być obiektem JSON.");
        return;
      }
      body = { rule, event: toCanonicalEvent(parsed) };
    } else {
      if (!eventId.trim()) {
        setError("Podaj event_id zdarzenia.");
        return;
      }
      body = { rule, event_id: eventId.trim() };
    }
    setBusy(true);
    try {
      const { data, error: apiError } = await api.POST("/rules/test", { body });
      if (apiError || !data) {
        setResult(null);
        setError(apiError ? `${apiError.error}${apiError.detail ? `: ${apiError.detail}` : ""}` : "Test nie powiódł się.");
      } else {
        setResult(data);
      }
    } catch {
      setResult(null);
      setError("Nie udało się połączyć z API.");
    } finally {
      setBusy(false);
    }
  }

  function insertSkeleton() {
    const paths = suggestedFields.length ? suggestedFields : ["schema", "raw.payload.example"];
    setText(JSON.stringify(skeletonFromFields(paths), null, 2));
  }

  return (
    <section className="rules-card" aria-labelledby="rt-title">
      <h2 id="rt-title">Testuj na zdarzeniu</h2>
      <p className="muted rules-lead">
        Sprawdź, czy reguła (także jeszcze niezapisana) pasuje do zdarzenia, i zobacz, który warunek zadziałał.
      </p>
      <div className="rules-seg" role="group" aria-label="Źródło zdarzenia">
        <button type="button" aria-pressed={mode === "paste"} onClick={() => setMode("paste")}>
          Wklej zdarzenie
        </button>
        <button type="button" aria-pressed={mode === "id"} onClick={() => setMode("id")}>
          Ostatnie zdarzenie (event_id)
        </button>
      </div>
      {mode === "paste" ? (
        <div className="rules-row">
          <label htmlFor="rt-event">Zdarzenie (JSON)</label>
          <textarea
            id="rt-event"
            className="mono"
            rows={8}
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            aria-describedby="rt-event-hint"
          />
          <div id="rt-event-hint" className="rules-field-hint">
            Wklej zdarzenie kanoniczne (z polem <code>raw</code>) albo sam payload – trafi do <code>raw</code>.{" "}
            <button type="button" className="rules-linkish" onClick={insertSkeleton}>
              Wstaw szkielet z podpowiedzi pól
            </button>
          </div>
        </div>
      ) : (
        <div className="rules-row">
          <label htmlFor="rt-event-id">event_id (UUID)</label>
          <input
            id="rt-event-id"
            className="mono"
            value={eventId}
            onChange={(e) => setEventId(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </div>
      )}
      <div className="rules-actions">
        <button type="button" className="primary" onClick={run} disabled={busy || !rule}>
          {busy ? "Testuję…" : "Uruchom test"}
        </button>
        {!rule && blockedReason && <span className="rules-field-hint">{blockedReason}</span>}
      </div>
      {error && (
        <p className="error-box" role="alert">
          {error}
        </p>
      )}
      {result && (
        <div className="rt-result" aria-live="polite">
          <p className={`rt-banner ${result.matched ? "rt-banner-pass" : "rt-banner-miss"}`}>
            <ResultIcon ok={result.matched} />
            {result.matched ? "Reguła pasuje do tego zdarzenia" : "Reguła NIE pasuje do tego zdarzenia"}
          </p>
          <h3>Wyjaśnienie</h3>
          <ul className="rt-tree">
            <ExplanationNode e={result.explanation} />
          </ul>
          <h3>Tak wyglądałoby powiadomienie</h3>
          <div className="rt-rendered">
            <div className="rt-rendered-title">
              {result.rendered.title || <span className="muted">(pusty tytuł)</span>}
            </div>
            {result.rendered.body && <div className="rt-rendered-body">{result.rendered.body}</div>}
            {result.rendered.link && <div className="mono rt-rendered-link">{result.rendered.link}</div>}
          </div>
        </div>
      )}
    </section>
  );
}
