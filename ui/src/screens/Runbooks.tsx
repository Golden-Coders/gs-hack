// Runbooki (tylko do odczytu): lista z plików konfiguracyjnych, warunek dopasowania, treść MANUAL / kroki AUTOMATIC.
// Edycja wyłącznie w plikach – tasks.md 15.4.
import { api, type Schemas } from "../api/client";
import { useResource } from "../api/useResource";
import { CodeBlock } from "@atlaskit/code";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import Lozenge from "@atlaskit/lozenge";
import SectionMessage from "@atlaskit/section-message";
import Markdown from "../components/Markdown";
import ConditionView from "../components/ConditionView";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
import { Empty, ErrorBox, Loading, PageHeader, StatusLozenge } from "../ui/primitives";
import "./Runbooks.css";

type Runbook = Schemas["RunbookDef"];
type Step = Schemas["StepDef"];

const STEP_TYPE: Record<Step["type"], string> = {
  check: "sprawdzenie",
  action: "akcja",
  manual: "krok ręczny",
  note: "notatka",
};

function StepItem({ step }: { step: Step }) {
  const hasParams = step.params && Object.keys(step.params).length > 0;
  return (
    <li className="step">
      <div>
        <div className="step-head">
          <span className="step-text">{step.text}</span>
          <Lozenge>{STEP_TYPE[step.type] ?? step.type}</Lozenge>
          <StatusLozenge domain="stepAutomation" value={step.automation} />
          {step.approval === "required" && <StatusLozenge domain="approval" value={step.approval} />}
          <span className="runbook-meta mono">{step.id}</span>
        </div>
        {step.action && (
          <div className="step-action">
            <Text size="small" color="color.text.subtle">akcja:</Text>
            <span className="mono">{step.action}</span>
          </div>
        )}
        {hasParams && <CodeBlock language="json" text={JSON.stringify(step.params, null, 2)} />}
        {step.condition && <div className="step-condition">Jeśli (tylko opis, nie jest wykonywane): {step.condition}</div>}
      </div>
    </li>
  );
}

function RunbookItem({ rb }: { rb: Runbook }) {
  const steps = rb.steps ?? [];
  const ruleIds = rb.rule_ids ?? [];
  const isManual = rb.kind === "manual";
  return (
    <details className="runbook">
      <summary>
        <span className="runbook-title">{rb.title}</span>
        <StatusLozenge domain="runbookKind" value={rb.kind} />
        <span className="runbook-meta mono">{rb.id}</span>
        {!isManual && <span className="runbook-meta">{steps.length} kroków</span>}
        {ruleIds.length > 0 && <span className="runbook-meta">reguły: {ruleIds.length}</span>}
      </summary>
      <div className="runbook-body">
        <SourceFile path={rb.source_file} />
        <section>
          <Heading size="xsmall" as="h3">Dopasowanie (matches)</Heading>
          <ConditionView condition={rb.matches} />
        </section>
        <section>
          <Heading size="xsmall" as="h3">Przypięte reguły (rule_ids)</Heading>
          {ruleIds.length ? (
            <div className="rule-chips">
              {ruleIds.map((id) => (
                <span key={id} className="rule-chip">
                  {id}
                </span>
              ))}
            </div>
          ) : (
            <Text color="color.text.subtle">brak – runbook dopasowuje się tylko po warunku</Text>
          )}
        </section>
        {isManual ? (
          <section>
            <Heading size="xsmall" as="h3">Treść (dołączana do powiadomień)</Heading>
            {rb.body ? <Markdown source={rb.body} /> : <Text color="color.text.subtle">brak treści</Text>}
          </section>
        ) : (
          <section>
            <Heading size="xsmall" as="h3">Kroki ({steps.length})</Heading>
            {steps.length ? (
              <ol className="steps">
                {steps.map((s) => (
                  <StepItem key={s.id} step={s} />
                ))}
              </ol>
            ) : (
              <Text color="color.text.subtle">brak kroków</Text>
            )}
          </section>
        )}
      </div>
    </details>
  );
}

export default function Runbooks() {
  const list = useResource(() => api.GET("/runbooks"));
  // Błędy parsowania są dodatkiem – ich brak nie blokuje listy.
  const loadErrors = useResource(() => api.GET("/runbooks/errors"));
  const runbooks = list.data;
  const errors = loadErrors.data ?? [];

  return (
    <section>
      <PageHeader>Runbooki</PageHeader>
      <ReadOnlyNote>
        Runbooki edytuje się w plikach konfiguracyjnych (wskazanych przy każdym runbooku); zmiany są ładowane bez restartu.
      </ReadOnlyNote>

      {errors.length > 0 && (
        <div role="alert" className="runbook-errors">
          <SectionMessage appearance="error" headingLevel="h2" title={`Nie załadowano plików runbooków (${errors.length})`}>
            <ul>
              {errors.map((e, i) => (
                <li key={i}>
                  <span className="mono">
                    {e.file}
                    {e.line != null ? `:${e.line}` : ""}
                  </span>
                  {" – "}
                  {e.error}
                </li>
              ))}
            </ul>
          </SectionMessage>
        </div>
      )}

      {list.error && <ErrorBox message={`Nie udało się pobrać runbooków: ${list.error}`} onRetry={list.reload} disabled={list.loading} />}
      {!list.error && runbooks === null && <Loading>Ładowanie runbooków…</Loading>}
      {runbooks && runbooks.length === 0 && <Empty>Brak runbooków w plikach konfiguracyjnych.</Empty>}
      {runbooks && runbooks.length > 0 && (
        <div className="runbook-list">
          {runbooks.map((rb) => (
            <RunbookItem key={rb.id} rb={rb} />
          ))}
        </div>
      )}
    </section>
  );
}
