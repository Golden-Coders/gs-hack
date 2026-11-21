// Przejrzysty, maksymalnie prosty ekran Runbooków z możliwością tworzenia/edycji na stronie
// oraz generowania kroków runbooka przez lokalny model AI (Ollama).
import { useEffect, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { ErrorMessage } from "@atlaskit/form";
import { Text } from "@atlaskit/primitives/compiled";
import { Sparkles } from "lucide-react";
import { api, API_URL, type Schemas } from "../api/client";
import { describeError } from "../api/request";
import { getPersonId, getSessionToken } from "../api/session";
import { useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { adsIcon } from "../ui/icons";
import {
  Card,
  Checkbox,
  ConfirmInline,
  Empty,
  ErrorBox,
  Field,
  Loading,
  PageHeader,
  Select,
  TextArea,
  TextInput,
} from "../ui/primitives";
import { slugify } from "./rules/model";
import "./Runbooks.css";

const SparklesIcon = adsIcon(Sparkles);

export function formatRedactedKinds(kinds: Record<string, number>): string {
  const entries = Object.entries(kinds);
  return entries.length ? entries.map(([k, v]) => `${k}: ${v}`).join(", ") : "";
}

type Runbook = Schemas["RunbookDef"];
type Step = Schemas["StepDef"];
type ActionDef = Schemas["ActionDef"];
type Rule = Schemas["RuleDef"];

interface EditableStep {
  uid: number;
  text: string;
  action: string;
  paramKey: string;
  paramValue: string;
  approval: "none" | "required";
}

let stepUid = 1;

function defaultParamKeyForAction(actionName: string, actions: ActionDef[]): string {
  if (!actionName) return "";
  if (actionName === "http_get") return "url";
  if (actionName === "escalate") return "team";
  if (actionName === "notify") return "people";
  if (actionName === "restart_service" || actionName === "flush_cache") return "service";
  if (actionName === "retry_workflow") return "workflowId";
  if (actionName === "open_ticket") return "summary";
  const found = actions.find((a) => a.name === actionName);
  const schema = found?.params_schema as { required?: string[]; properties?: Record<string, unknown> } | null | undefined;
  if (schema?.required?.length) return schema.required[0];
  if (schema?.properties) {
    const keys = Object.keys(schema.properties);
    if (keys.length > 0) return keys[0];
  }
  return "value";
}

function stepToEditable(s: Step, actions: ActionDef[]): EditableStep {
  const action = s.action ?? "";
  const entries = Object.entries(s.params ?? {});
  const [firstKey, firstVal] = entries[0] ?? [defaultParamKeyForAction(action, actions), ""];
  const paramValue =
    firstVal === undefined || firstVal === null
      ? ""
      : Array.isArray(firstVal)
        ? firstVal.join(", ")
        : typeof firstVal === "string"
          ? firstVal
          : JSON.stringify(firstVal);
  return {
    uid: stepUid++,
    text: s.text ?? "",
    action,
    paramKey: firstKey || defaultParamKeyForAction(action, actions),
    paramValue,
    approval: s.approval === "required" ? "required" : "none",
  };
}

function editableToStep(es: EditableStep, idx: number, actions: ActionDef[]): Step {
  const action = es.action.trim();
  const params: Record<string, unknown> = {};
  const key = (es.paramKey.trim() || defaultParamKeyForAction(action, actions)).trim();
  const val = es.paramValue.trim();
  if (action && key && val) {
    if (action === "notify" && key === "people") {
      params[key] = val.split(",").map((x) => x.trim()).filter(Boolean);
    } else {
      params[key] = val;
    }
  }
  return {
    id: `s${idx + 1}`,
    type: action === "http_get" ? "check" : action ? "action" : "manual",
    text: es.text.trim() || (action ? `Wykonaj ${action}` : `Krok ${idx + 1}`),
    action: action || null,
    params,
    approval: es.approval,
    condition: null,
    automation: action ? "automated" : "manual",
  };
}

function emptyEditableStep(): EditableStep {
  return {
    uid: stepUid++,
    text: "",
    action: "",
    paramKey: "",
    paramValue: "",
    approval: "none",
  };
}

export default function Runbooks() {
  const listRes = useResource(() => api.GET("/runbooks"));
  const actionsRes = useResource(() => api.GET("/actions"));
  const rulesRes = useResource(() => api.GET("/rules"));

  const runbooks: Runbook[] = listRes.data ?? [];
  const actions: ActionDef[] = actionsRes.data ?? [];
  const rules: Rule[] = rulesRes.data ?? [];

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isCreatingNew, setIsCreatingNew] = useState(false);

  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "config_changed") {
      listRes.refresh();
      actionsRes.refresh();
      rulesRes.refresh();
    }
  });

  useEffect(() => {
    if (selectedId !== null || isCreatingNew) return;
    if (runbooks.length > 0) {
      setSelectedId(runbooks[0].id);
    }
  }, [runbooks, selectedId, isCreatingNew]);

  const activeRunbook = isCreatingNew
    ? null
    : selectedId
      ? (runbooks.find((r) => r.id === selectedId) ?? null)
      : null;

  function handleSelect(rb: Runbook) {
    setIsCreatingNew(false);
    setSelectedId(rb.id);
  }

  function handleStartNew() {
    setSelectedId(null);
    setIsCreatingNew(true);
  }

  function handleSaved(saved: Runbook) {
    setIsCreatingNew(false);
    setSelectedId(saved.id);
    listRes.refresh();
  }

  async function handleDelete(id: string): Promise<string | null> {
    const sess = getSessionToken();
    const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
    const headers: Record<string, string> = { Authorization: `Bearer ${bearer}` };
    const personId = getPersonId();
    if (personId) headers["X-Person-Id"] = personId;

    const resp = await fetch(`${API_URL}/runbooks/${encodeURIComponent(id)}`, {
      method: "DELETE",
      credentials: "include",
      headers,
    });
    if (!resp.ok) {
      const errBody = await resp.json().catch(() => null);
      return describeError(errBody, resp);
    }
    const remaining = runbooks.filter((r) => r.id !== id);
    setSelectedId(remaining[0]?.id ?? null);
    setIsCreatingNew(false);
    listRes.refresh();
    return null;
  }

  return (
    <section className="rb-screen">
      <PageHeader
        actions={
          <Button appearance="primary" onClick={handleStartNew}>
            + Nowy runbook
          </Button>
        }
      >
        Runbooki
      </PageHeader>

      {listRes.error && <ErrorBox message={listRes.error} onRetry={listRes.reload} />}

      <div className="rb-layout">
        {/* Lewa kolumna: lista runbooków */}
        <aside className="rb-sidebar" aria-label="Lista runbooków">
          {listRes.loading && listRes.data === null && <Loading />}
          {!listRes.loading && runbooks.length === 0 && (
            <Empty>Brak runbooków. Kliknij „+ Nowy runbook”.</Empty>
          )}
          <ul className="rb-list">
            {runbooks.map((rb) => {
              const active = !isCreatingNew && rb.id === selectedId;
              const stepsCount = rb.steps?.length ?? 0;
              return (
                <li key={rb.id}>
                  <button
                    type="button"
                    className={`rb-item ${active ? "active" : ""}`}
                    onClick={() => handleSelect(rb)}
                  >
                    <div className="rb-item-title">{rb.title}</div>
                    <div className="rb-item-sub">
                      {stepsCount > 0 ? `Kroków: ${stepsCount}` : "Instrukcja tekstowa"}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>

        {/* Prawa kolumna: edytor runbooka z AI */}
        <div className="rb-main">
          {isCreatingNew || activeRunbook ? (
            <RunbookEditor
              key={isCreatingNew ? "__new__" : activeRunbook?.id}
              initial={activeRunbook}
              actions={actions}
              rules={rules}
              onSaved={handleSaved}
              onDelete={handleDelete}
            />
          ) : (
            <Card>
              <Heading size="small" as="h2">
                Wybierz runbook z listy po lewej stronie
              </Heading>
            </Card>
          )}
        </div>
      </div>
    </section>
  );
}

function RunbookEditor({
  initial,
  actions,
  rules,
  onSaved,
  onDelete,
}: {
  initial: Runbook | null;
  actions: ActionDef[];
  rules: Rule[];
  onSaved: (rb: Runbook) => void;
  onDelete: (id: string) => Promise<string | null>;
}) {
  const isNew = initial === null;
  const [title, setTitle] = useState(initial?.title ?? "");
  const [ruleIds, setRuleIds] = useState<string[]>(initial?.rule_ids ?? []);
  const [steps, setSteps] = useState<EditableStep[]>(() => {
    if (initial?.steps && initial.steps.length > 0) {
      return initial.steps.map((s) => stepToEditable(s, actions));
    }
    if (initial?.body) {
      return [
        {
          uid: stepUid++,
          text: initial.body.trim(),
          action: "",
          paramKey: "",
          paramValue: "",
          approval: "none",
        },
      ];
    }
    return [emptyEditableStep()];
  });

  // Panel AI (połączony z lokalnym modelem Ollama przez POST /runbooks/draft)
  const [aiPrompt, setAiPrompt] = useState(initial?.kind === "manual" && initial?.body ? initial.body : "");
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [aiInfo, setAiInfo] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  function toggleRule(ruleId: string) {
    setRuleIds((prev) => (prev.includes(ruleId) ? prev.filter((id) => id !== ruleId) : [...prev, ruleId]));
  }

  function updateStep(uid: number, patch: Partial<EditableStep>) {
    setSteps((prev) =>
      prev.map((s) => {
        if (s.uid !== uid) return s;
        const next = { ...s, ...patch };
        if (patch.action !== undefined) {
          next.paramKey = defaultParamKeyForAction(patch.action, actions);
          const actDef = actions.find((a) => a.name === patch.action);
          if (actDef?.risk === "high") {
            next.approval = "required";
          }
        }
        return next;
      }),
    );
  }

  function addStep() {
    setSteps((prev) => [...prev, emptyEditableStep()]);
  }

  function removeStep(uid: number) {
    setSteps((prev) => (prev.length <= 1 ? [emptyEditableStep()] : prev.filter((s) => s.uid !== uid)));
  }

  async function handleAiGenerate() {
    const text = aiPrompt.trim();
    if (!text || aiBusy) return;
    setAiBusy(true);
    setAiError(null);
    setAiInfo(null);
    try {
      const { data, error, response } = await api.POST("/runbooks/draft", {
        body: {
          id: initial?.id ?? (slugify(title) || null),
          title: title.trim() || null,
          text,
          rule_ids: ruleIds,
          matches: initial?.matches ?? null,
        },
      });
      if (error !== undefined || !response.ok || !data) {
        setAiError(describeError(error, response));
        return;
      }
      if (data.draft.title && (!title.trim() || isNew)) {
        setTitle(data.draft.title);
      }
      if (data.draft.steps && data.draft.steps.length > 0) {
        setSteps(data.draft.steps.map((s) => stepToEditable(s, actions)));
      }
      setAiInfo(`Wygenerowano kroków z AI: ${data.draft.steps?.length ?? 0}`);
    } catch {
      setAiError("Nie udało się połączyć z lokalnym modelem AI (Ollama).");
    } finally {
      setAiBusy(false);
    }
  }

  async function handleSave() {
    setSaveError(null);
    const cleanTitle = title.trim();
    if (!cleanTitle) {
      setSaveError("Podaj nazwę runbooka.");
      return;
    }
    const builtSteps = steps
      .filter((s) => s.text.trim() !== "" || s.action.trim() !== "")
      .map((s, idx) => editableToStep(s, idx, actions));
    if (builtSteps.length === 0) {
      setSaveError("Dodaj przynajmniej jeden krok runbooka.");
      return;
    }

    const id = isNew ? slugify(cleanTitle) || `runbook-${Date.now()}` : initial.id;
    const payload: Runbook = {
      id,
      title: cleanTitle,
      kind: "automatic",
      matches: initial?.matches ?? null,
      rule_ids: ruleIds,
      body: null,
      steps: builtSteps,
      source_file: null,
      version: initial?.version ?? 1,
    };

    setSaving(true);
    try {
      const sess = getSessionToken();
      const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
        Authorization: `Bearer ${bearer}`,
      };
      const personId = getPersonId();
      if (personId) headers["X-Person-Id"] = personId;

      const url = isNew ? `${API_URL}/runbooks` : `${API_URL}/runbooks/${encodeURIComponent(initial.id)}`;
      const method = isNew ? "POST" : "PUT";

      const resp = await fetch(url, {
        method,
        credentials: "include",
        headers,
        body: JSON.stringify(payload),
      });
      if (!resp.ok) {
        const errBody = await resp.json().catch(() => null);
        setSaveError(describeError(errBody, resp));
        return;
      }
      const saved = (await resp.json()) as Runbook;
      onSaved(saved);
    } catch {
      setSaveError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    if (!initial) return;
    setSaving(true);
    setSaveError(null);
    const err = await onDelete(initial.id);
    if (err) setSaveError(`Nie udało się usunąć runbooka: ${err}`);
    setSaving(false);
  }

  const actionOptions = [
    { value: "", label: "Krok ręczny (bez automatyzacji)" },
    ...actions.map((a) => ({
      value: a.name,
      label: `${a.name}${a.description ? ` — ${a.description}` : ""}`,
    })),
  ];

  return (
    <div className="rb-editor">
      <header className="rb-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? "Nowy runbook" : title || initial.title}
        </Heading>
      </header>

      {/* 1. Asystent AI */}
      <Card title="Asystent AI — wygeneruj runbook z opisu lub instrukcji">
        <div className="rb-ai-box">
          <Field
            label="Wklej opis procedury lub napisz własnymi słowami, co ma robić runbook"
            hint="AI automatycznie rozbije tekst na kroki i dobierze odpowiednie akcje z systemu (np. http_get, restart_service, retry_workflow, escalate)."
          >
            {(f) => (
              <TextArea
                {...f}
                minimumRows={3}
                value={aiPrompt}
                onChange={(e) => {
                  setAiPrompt(e.currentTarget.value);
                  if (aiError) setAiError(null);
                }}
                placeholder="Np. 1. Sprawdź czy serwis płatności działa. 2. Zrestartuj usługę payments-api. 3. Jeśli nadal nie działa, eskaluj do zespołu payments."
              />
            )}
          </Field>
          <div className="rb-ai-actions">
            <Button
              appearance="primary"
              iconBefore={SparklesIcon}
              isDisabled={!aiPrompt.trim() || aiBusy}
              isLoading={aiBusy}
              onClick={() => void handleAiGenerate()}
            >
              Wygeneruj kroki z AI
            </Button>
            {aiInfo && (
              <Text size="small" color="color.text.subtle">
                {aiInfo}
              </Text>
            )}
            {aiError && <ErrorMessage>{aiError}</ErrorMessage>}
          </div>
        </div>
      </Card>

      {/* 2. Nazwa i powiązane reguły */}
      <Card title="1. Nazwa i powiązane reguły">
        <Field label="Nazwa runbooka">
          {(f) => (
            <TextInput
              {...f}
              value={title}
              onChange={(e) => setTitle(e.currentTarget.value)}
              placeholder="np. Procedura naprawy błędu płatności"
            />
          )}
        </Field>

        {rules.length > 0 && (
          <div className="rb-rules-box">
            <Text weight="semibold" size="small">
              Dla jakich reguł uruchamiać ten runbook? (opcjonalnie)
            </Text>
            <div className="rb-rules-grid">
              {rules.map((r) => (
                <Checkbox
                  key={r.id}
                  isChecked={ruleIds.includes(r.id)}
                  onChange={() => toggleRule(r.id)}
                  label={r.name}
                />
              ))}
            </div>
          </div>
        )}
      </Card>

      {/* 3. Lista kroków */}
      <Card title="2. Kroki runbooka">
        <div className="rb-steps-list">
          {steps.map((s, idx) => {
            const pKey = s.paramKey || defaultParamKeyForAction(s.action, actions);
            return (
              <div key={s.uid} className="rb-step-card">
                <div className="rb-step-top">
                  <span className="rb-step-num">Krok {idx + 1}</span>
                  <Button appearance="subtle" onClick={() => removeStep(s.uid)}>
                    Usuń krok
                  </Button>
                </div>

                <div className="rb-step-grid">
                  <Field label="Co należy zrobić? (opis kroku)">
                    {(f) => (
                      <TextInput
                        {...f}
                        value={s.text}
                        onChange={(e) => updateStep(s.uid, { text: e.currentTarget.value })}
                        placeholder="np. Zrestartuj usługę lub sprawdź status"
                      />
                    )}
                  </Field>

                  <Field label="Akcja do wykonania">
                    {(f) => (
                      <Select
                        {...f}
                        value={s.action}
                        options={actionOptions}
                        onChange={(val) => updateStep(s.uid, { action: val })}
                      />
                    )}
                  </Field>
                </div>

                {s.action && (
                  <div className="rb-step-extra">
                    <Field
                      label={`Parametr akcji (${pKey || "wartość"})`}
                      hint="Np. nazwa usługi, adres URL, zespół lub zmienna {{raw.workflowId}}"
                    >
                      {(f) => (
                        <TextInput
                          {...f}
                          value={s.paramValue}
                          onChange={(e) => updateStep(s.uid, { paramKey: pKey, paramValue: e.currentTarget.value })}
                          placeholder={
                            pKey === "url"
                              ? "http://mock-targets:8090/health"
                              : pKey === "team"
                                ? "np. payments"
                                : "np. {{service}}"
                          }
                        />
                      )}
                    </Field>

                    <div className="rb-step-approval">
                      <Checkbox
                        isChecked={s.approval === "required"}
                        onChange={() =>
                          updateStep(s.uid, {
                            approval: s.approval === "required" ? "none" : "required",
                          })
                        }
                        label="Wymagaj zatwierdzenia człowieka przed wykonaniem tego kroku"
                      />
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <div className="rb-add-btn">
          <Button onClick={addStep}>+ Dodaj kolejny krok</Button>
        </div>
      </Card>

      {saveError && <ErrorBox message={saveError} />}

      <div className="rb-savebar">
        <Button appearance="primary" isLoading={saving} isDisabled={saving} onClick={() => void handleSave()}>
          {isNew ? "Utwórz runbook" : "Zapisz runbook"}
        </Button>
        {!isNew && (
          <ConfirmInline
            label="Usuń runbook"
            question={`Usunąć runbook „${initial.title}”?`}
            confirmLabel="Tak, usuń"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </div>
  );
}
