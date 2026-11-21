import { useState } from "react";
import Button from "@atlaskit/button/new";
import { Check, X } from "lucide-react";
import Code from "@atlaskit/code/inline";
import Heading from "@atlaskit/heading";
import SectionMessage from "@atlaskit/section-message";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../../api/client";
import { describeError } from "../../api/request";
import { formatValue } from "../../components/Presentation";
import { Card, ErrorBox, Field, Segmented, TextArea, TextInput } from "../../ui/primitives";
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
  return ok ? <Check className="rt-icon rt-ok" size={16} aria-hidden /> : <X className="rt-icon rt-fail" size={16} aria-hidden />;
}

function ExplanationNode({ e }: { e: Explanation }) {
  const label = nodeLabel(e.node);
  return (
    <li>
      <div className={`rt-node ${e.result ? "rt-pass" : "rt-miss"}`}>
        <ResultIcon ok={e.result} />
        {label.kind === "group" ? <span className="rt-group-label">{label.text}</span> : <Code>{label.text}</Code>}
        <span className="rt-verdict">{e.result ? "spełniony" : "niespełniony"}</span>
        {label.kind === "leaf" && e.actual !== undefined && (
          <Text size="small" color="color.text.subtle">
            w zdarzeniu: <Code>{e.actual === null ? "brak pola" : formatValue(e.actual)}</Code>
          </Text>
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
      const { data, error: apiError, response } = await api.POST("/rules/test", { body });
      if (apiError || !data) {
        setResult(null);
        setError(apiError ? describeError(apiError, response) : "Test nie powiódł się.");
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
    <Card title="Testuj na zdarzeniu" className="rules-tester">
      <Text as="p" color="color.text.subtle">
        Sprawdź, czy reguła (także jeszcze niezapisana) pasuje do zdarzenia, i zobacz, który warunek zadziałał.
      </Text>
      <Segmented
        label="Źródło zdarzenia"
        value={mode}
        onChange={setMode}
        options={[
          { value: "paste", label: "Wklej zdarzenie" },
          { value: "id", label: "Ostatnie zdarzenie (event_id)" },
        ]}
      />
      {mode === "paste" ? (
        <Field
          label="Zdarzenie (JSON)"
          hint={
            <>
              Wklej zdarzenie kanoniczne (z polem <Code>raw</Code>) albo sam payload – trafi do <Code>raw</Code>.{" "}
              <Button appearance="subtle" spacing="compact" onClick={insertSkeleton}>
                Wstaw szkielet z podpowiedzi pól
              </Button>
            </>
          }
        >
          {(f) => <TextArea {...f} isMonospaced minimumRows={8} value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />}
        </Field>
      ) : (
        <Field label="event_id (UUID)">
          {(f) => (
            <TextInput {...f} isMonospaced value={eventId} onChange={(e) => setEventId(e.currentTarget.value)} spellCheck={false} autoComplete="off" />
          )}
        </Field>
      )}
      <div className="rules-actions">
        <Button appearance="primary" onClick={run} isDisabled={!rule} isLoading={busy}>
          Uruchom test
        </Button>
        {!rule && blockedReason && (
          <Text size="small" color="color.text.subtle">
            {blockedReason}
          </Text>
        )}
      </div>
      {error && <ErrorBox message={error} />}
      {result && (
        <div className="rt-result" aria-live="polite">
          <SectionMessage appearance={result.matched ? "success" : "warning"}>
            {result.matched ? "Reguła pasuje do tego zdarzenia" : "Reguła NIE pasuje do tego zdarzenia"}
          </SectionMessage>
          <Heading size="xsmall" as="h3">
            Wyjaśnienie
          </Heading>
          <ul className="rt-tree">
            <ExplanationNode e={result.explanation} />
          </ul>
          <Heading size="xsmall" as="h3">
            Tak wyglądałoby powiadomienie
          </Heading>
          <div className="rt-rendered">
            <div className="rt-rendered-title">
              {result.rendered.title || <Text color="color.text.subtle">(pusty tytuł)</Text>}
            </div>
            {result.rendered.body && <div className="rt-rendered-body">{result.rendered.body}</div>}
            {result.rendered.link && <Code>{result.rendered.link}</Code>}
          </div>
        </div>
      )}
    </Card>
  );
}
