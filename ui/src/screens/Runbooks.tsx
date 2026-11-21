// Runbooki (tylko do odczytu): lista z plików konfiguracyjnych, warunek dopasowania, treść MANUAL / kroki AUTOMATIC.
// Edycja wyłącznie w plikach – tasks.md 15.4.
import { api, type Schemas } from "../api/client";
import { useResource } from "../api/useResource";
import { Badge } from "../components/Badges";
import Markdown from "../components/Markdown";
import ConditionView from "../components/ConditionView";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
import { Empty, ErrorBox, Loading } from "../ui/primitives";
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
          <Badge tone="gray">{STEP_TYPE[step.type] ?? step.type}</Badge>
          <Badge tone={step.automation === "automated" ? "blue" : "yellow"}>
            {step.automation === "automated" ? "automatyczny" : "ręczny"}
          </Badge>
          {step.approval === "required" && <Badge tone="orange">wymaga zatwierdzenia</Badge>}
          <span className="runbook-meta mono">{step.id}</span>
        </div>
        {step.action && (
          <div className="step-action">
            <span className="runbook-meta">akcja:</span>
            <span className="mono">{step.action}</span>
          </div>
        )}
        {hasParams && <pre className="step-params">{JSON.stringify(step.params, null, 2)}</pre>}
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
        <Badge tone={isManual ? "gray" : "blue"}>{isManual ? "ręczny" : "automatyczny"}</Badge>
        <span className="runbook-meta mono">{rb.id}</span>
        {!isManual && <span className="runbook-meta">{steps.length} kroków</span>}
        {ruleIds.length > 0 && <span className="runbook-meta">reguły: {ruleIds.length}</span>}
      </summary>
      <div className="runbook-body">
        <SourceFile path={rb.source_file} />
        <section>
          <h3>Dopasowanie (matches)</h3>
          <ConditionView condition={rb.matches} />
        </section>
        <section>
          <h3>Przypięte reguły (rule_ids)</h3>
          {ruleIds.length ? (
            <div className="rule-chips">
              {ruleIds.map((id) => (
                <span key={id} className="rule-chip">
                  {id}
                </span>
              ))}
            </div>
          ) : (
            <span className="muted">brak – runbook dopasowuje się tylko po warunku</span>
          )}
        </section>
        {isManual ? (
          <section>
            <h3>Treść (dołączana do powiadomień)</h3>
            {rb.body ? <Markdown source={rb.body} /> : <span className="muted">brak treści</span>}
          </section>
        ) : (
          <section>
            <h3>Kroki ({steps.length})</h3>
            {steps.length ? (
              <ol className="steps">
                {steps.map((s) => (
                  <StepItem key={s.id} step={s} />
                ))}
              </ol>
            ) : (
              <span className="muted">brak kroków</span>
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
      <h1>Runbooki</h1>
      <ReadOnlyNote>
        Runbooki edytuje się w plikach konfiguracyjnych (wskazanych przy każdym runbooku); zmiany są ładowane bez restartu.
      </ReadOnlyNote>

      {errors.length > 0 && (
        <div className="error-box runbook-errors" role="alert">
          <strong>Nie załadowano plików runbooków ({errors.length}):</strong>
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
