// Runbooki: lista z plików i bazy, oraz opcjonalna ekstrakcja kroków AUTOMATIC z tekstu/Markdownu przez LLM.
import { useState } from "react";
import { api, type Schemas } from "../api/client";
import { describeError } from "../api/request";
import { useResource } from "../api/useResource";
import { CodeBlock } from "@atlaskit/code";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import Lozenge from "@atlaskit/lozenge";
import SectionMessage from "@atlaskit/section-message";
import Markdown from "../components/Markdown";
import ConditionView from "../components/ConditionView";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
import { Card, CopyButton, Empty, ErrorBox, Field, Loading, Tabs } from "../ui/primitives";
import "./Runbooks.css";

type Runbook = Schemas["RunbookDef"];
type Step = Schemas["StepDef"];
type DraftResponse = Schemas["RunbookDraftResponse"];

const STEP_TYPE: Record<Step["type"], string> = {
  check: "sprawdzenie",
  action: "akcja",
  manual: "krok ręczny",
  note: "notatka",
};

function StepItem({ step, onRemove }: { step: Step; onRemove?: () => void }) {
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
          {onRemove && (
            <button type="button" className="step-remove" onClick={onRemove}>
              Usuń
            </button>
          )}
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

function RunbookItem({
  rb,
  llmEnabled,
  extractingId,
  onExtract,
}: {
  rb: Runbook;
  llmEnabled: boolean;
  extractingId: string | null;
  onExtract: (rb: Runbook) => void;
}) {
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
            <div className="runbook-section-head">
              <h3>Treść (dołączana do powiadomień)</h3>
              <button
                type="button"
                disabled={!llmEnabled || extractingId !== null}
                title={llmEnabled ? "Wyodrębnij kroki AUTOMATIC przez LLM (z podglądem przed zapisem)" : "Ustaw LLM_PROVIDER=local lub api"}
                onClick={() => onExtract(rb)}
              >
                {extractingId === rb.id ? "Ekstrakcja…" : "Wyodrębnij kroki (LLM)"}
              </button>
            </div>
            {rb.body ? <Markdown source={rb.body} /> : <span className="muted">brak treści</span>}
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

export function formatRedactedKinds(kinds: Record<string, number>): string {
  const entries = Object.entries(kinds);
  return entries.length ? entries.map(([k, v]) => `${k}: ${v}`).join(", ") : "";
}

export default function Runbooks() {
  const list = useResource(() => api.GET("/runbooks"));
  // Błędy parsowania i status LLM są dodatkami – ich brak nie blokuje listy.
  const loadErrors = useResource(() => api.GET("/runbooks/errors"));
  const authorStatus = useResource(() => api.GET("/runbooks/author/status"));

  const [showImport, setShowImport] = useState(false);
  const [importId, setImportId] = useState("");
  const [importTitle, setImportTitle] = useState("");
  const [importText, setImportText] = useState("");

  const [extractingId, setExtractingId] = useState<string | null>(null);
  const [draftError, setDraftError] = useState<string | null>(null);
  const [draftResult, setDraftResult] = useState<DraftResponse | null>(null);
  const [draftTab, setDraftTab] = useState<"steps" | "yaml">("steps");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const runbooks = list.data;
  const errors = loadErrors.data ?? [];
  const llm = authorStatus.data;
  const llmEnabled = llm?.enabled === true;

  async function requestDraft(body: Schemas["RunbookDraftRequest"], sourceKey: string) {
    setExtractingId(sourceKey);
    setDraftError(null);
    setSaveError(null);
    try {
      const { data, error, response } = await api.POST("/runbooks/draft", { body });
      if (error !== undefined || !response.ok || !data) {
        setDraftError(describeError(error, response));
        return;
      }
      setDraftResult(data);
      setDraftTab("steps");
      setShowImport(false);
    } catch {
      setDraftError("Brak połączenia z API");
    } finally {
      setExtractingId(null);
    }
  }

  async function handleSaveDraft() {
    if (!draftResult) return;
    setSaving(true);
    setSaveError(null);
    try {
      const { error, response } = await api.POST("/runbooks", { body: draftResult.draft });
      if (error !== undefined || !response.ok) {
        setSaveError(describeError(error, response));
        return;
      }
      setDraftResult(null);
      list.reload();
    } catch {
      setSaveError("Brak połączenia z API");
    } finally {
      setSaving(false);
    }
  }

  function updateDraft(patch: Partial<Runbook>) {
    setDraftResult((prev) => (prev ? { ...prev, draft: { ...prev.draft, ...patch } } : null));
  }

  function removeDraftStep(stepId: string) {
    setDraftResult((prev) => {
      if (!prev) return null;
      const steps = (prev.draft.steps ?? []).filter((s) => s.id !== stepId);
      return { ...prev, draft: { ...prev.draft, steps } };
    });
  }

  return (
    <section>
      <div className="runbooks-header">
        <h1>Runbooki</h1>
        <button
          type="button"
          disabled={!llmEnabled}
          title={llmEnabled ? "Wklej Markdown lub tekst i wyodrębnij kroki przez LLM" : "Wymaga LLM_PROVIDER=local lub api"}
          onClick={() => {
            setShowImport((v) => !v);
            setDraftError(null);
          }}
        >
          {showImport ? "Ukryj import LLM" : "Importuj z tekstu (LLM)"}
        </button>
      </div>

      <ReadOnlyNote>
        Runbooki z plików konfiguracyjnych są w trybie tylko do odczytu (zmiany w plikach ładowane bez restartu).{" "}
        {llm ? (
          llm.enabled ? (
            <>
              Ekstrakcja LLM aktywna: <span className="mono">{llm.provider}</span>
              {llm.model ? (
                <>
                  {" "}
                  (<span className="mono">{llm.model}</span>)
                </>
              ) : null}
              .
            </>
          ) : (
            <>Ekstrakcja LLM jest wyłączona (ustaw LLM_PROVIDER=local lub api).</>
          )
        ) : null}
      </ReadOnlyNote>

      {draftError && <ErrorBox message={`Błąd ekstrakcji LLM: ${draftError}`} />}

      {showImport && llmEnabled && (
        <Card title="Importuj runbook z tekstu (LLM)" className="llm-panel">
          <div className="llm-form-grid">
            <Field label="ID runbooka (opcjonalnie)" hint="np. payments-recovery">
              {(props) => <input {...props} value={importId} onChange={(e) => setImportId(e.target.value)} placeholder="wygenerowane z tytułu" />}
            </Field>
            <Field label="Tytuł (opcjonalnie)">
              {(props) => <input {...props} value={importTitle} onChange={(e) => setImportTitle(e.target.value)} placeholder="wyodrębniony z tekstu" />}
            </Field>
          </div>
          <Field
            label="Treść runbooka (Markdown lub wolny tekst)"
            hint="Przed wysłaniem do modelu adresy URL, IP, e-maile i hosty wewnętrzne zostaną zredagowane; zmienne {{...}} pozostaną bez zmian."
          >
            {(props) => (
              <textarea
                {...props}
                rows={8}
                value={importText}
                onChange={(e) => setImportText(e.target.value)}
                placeholder={"# Settlement timeout\n1. Check health of service {{raw.service}}\n2. Retry workflow {{raw.workflowId}}"}
              />
            )}
          </Field>
          <div className="llm-actions">
            <button
              type="button"
              className="primary"
              disabled={extractingId !== null || !importText.trim()}
              onClick={() =>
                requestDraft(
                  {
                    id: importId.trim() || null,
                    title: importTitle.trim() || null,
                    text: importText,
                  },
                  "__import__",
                )
              }
            >
              {extractingId === "__import__" ? "Ekstrakcja…" : "Wyodrębnij kroki"}
            </button>
            <button type="button" onClick={() => setShowImport(false)} disabled={extractingId !== null}>
              Anuluj
            </button>
          </div>
        </Card>
      )}

      {draftResult && (
        <Card
          title="Podgląd szkicu runbooka (LLM)"
          className="llm-panel"
          actions={
            <Tabs
              label="Widok szkicu"
              value={draftTab}
              onChange={setDraftTab}
              tabs={[
                { value: "steps", label: `Kroki (${draftResult.draft.steps?.length ?? 0})` },
                { value: "yaml", label: "YAML" },
              ]}
            />
          }
        >
          <div className="llm-form-grid">
            <Field label="ID runbooka">
              {(props) => (
                <input
                  {...props}
                  value={draftResult.draft.id}
                  onChange={(e) => updateDraft({ id: e.target.value })}
                />
              )}
            </Field>
            <Field label="Tytuł">
              {(props) => (
                <input
                  {...props}
                  value={draftResult.draft.title}
                  onChange={(e) => updateDraft({ title: e.target.value })}
                />
              )}
            </Field>
          </div>

          <div className="llm-meta">
            {draftResult.redacted_count > 0 ? (
              <Badge tone="blue">
                {`Zredagowano przed wysłaniem: ${draftResult.redacted_count} (${formatRedactedKinds(draftResult.redacted_kinds)})`}
              </Badge>
            ) : (
              <Badge tone="gray">Brak danych wrażliwych do redakcji</Badge>
            )}
          </div>

          {draftResult.warnings.length > 0 && (
            <div className="llm-warnings" role="status">
              <strong>Uwagi klasyfikacji rejestru:</strong>
              <ul>
                {draftResult.warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            </div>
          )}

          {draftTab === "steps" ? (
            (draftResult.draft.steps?.length ?? 0) > 0 ? (
              <ol className="steps">
                {(draftResult.draft.steps ?? []).map((s) => (
                  <StepItem
                    key={s.id}
                    step={s}
                    onRemove={draftResult.draft.steps && draftResult.draft.steps.length > 1 ? () => removeDraftStep(s.id) : undefined}
                  />
                ))}
              </ol>
            ) : (
              <Empty>Brak kroków w szkicu.</Empty>
            )
          ) : (
            <div>
              <div className="llm-yaml-bar">
                <CopyButton text={draftResult.yaml} label="Kopiuj YAML" />
              </div>
              <pre className="step-params">{draftResult.yaml}</pre>
            </div>
          )}

          {saveError && <ErrorBox message={`Nie udało się zapisać runbooka: ${saveError}`} />}

          <div className="llm-actions">
            <button
              type="button"
              className="primary"
              disabled={saving || !draftResult.draft.id.trim() || !(draftResult.draft.steps?.length)}
              onClick={handleSaveDraft}
            >
              {saving ? "Zapisywanie…" : "Zapisz runbook"}
            </button>
            <button type="button" onClick={() => setDraftResult(null)} disabled={saving}>
              Anuluj
            </button>
          </div>
        </Card>
      )}

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
            <RunbookItem
              key={rb.id}
              rb={rb}
              llmEnabled={llmEnabled}
              extractingId={extractingId}
              onExtract={(item) => requestDraft({ runbook_id: item.id }, item.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}
