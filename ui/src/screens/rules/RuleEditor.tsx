import { useEffect, useMemo, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { ErrorMessage } from "@atlaskit/form";
import SectionMessage from "@atlaskit/section-message";
import { Text } from "@atlaskit/primitives/compiled";
import { Sparkles } from "lucide-react";
import { api, API_URL, type Schemas } from "../../api/client";
import { describeError, send } from "../../api/request";
import { getPersonId, getSessionToken } from "../../api/session";
import { adsIcon } from "../../ui/icons";
import { Card, Checkbox, ConfirmInline, ErrorBox, Field, Segmented, Select, TextArea, TextInput } from "../../ui/primitives";
import {
  CANONICAL_FIELDS,
  OPS,
  SEVERITIES,
  emptyForm,
  findLeaves,
  formToRule,
  newLeaf,
  ruleToForm,
  slugify,
  validateForm,
  type LeafNode,
  type Op,
  type Recipient,
  type Rule,
  type RuleForm,
} from "./model";

const SparklesIcon = adsIcon(Sparkles);

type Person = Schemas["PersonDef"];
type Team = Schemas["TeamDef"];

const FIELD_LIST_ID = "rule-field-options";

const COMMON_FIELDS = [
  ...CANONICAL_FIELDS,
  "raw.type",
  "raw.error.code",
  "raw.workflowName",
  "raw.alertname",
  "raw.labels.service",
  "raw.labels.region",
  "raw.payload.desk",
  "raw.payload.reason",
  "raw.payload.notional",
  "service",
];

interface Props {
  initial: Rule | null;
  people: Person[];
  teams: Team[];
  schemas: string[];
  onSaved: (rule: Rule, wasNew: boolean) => void;
  onDeleted: (id: string) => void;
  onDirtyChange: (dirty: boolean) => void;
}

function apiMessage(error: unknown, response: Response): string {
  const line = (error as { line?: number | null } | undefined)?.line;
  return `${describeError(error, response)}${line ? ` (line ${line})` : ""}`;
}

export default function RuleEditor({ initial, people, teams, onSaved, onDeleted, onDirtyChange }: Props) {
  const isNew = initial === null;
  const [initialForm] = useState<RuleForm>(() => (initial ? ruleToForm(initial) : emptyForm()));
  const [form, setForm] = useState<RuleForm>(initialForm);
  const [baseline, setBaseline] = useState(() => JSON.stringify(formToRule(initialForm)));
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [aiPrompt, setAiPrompt] = useState("");
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [aiToolInfo, setAiToolInfo] = useState<string | null>(null);

  const rule = useMemo(() => formToRule(form), [form]);
  const snapshot = useMemo(() => JSON.stringify(rule), [rule]);
  const dirty = snapshot !== baseline;
  const errors = useMemo(() => validateForm(form, isNew), [form, isNew]);

  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  const update = (patch: Partial<RuleForm>) => setForm((f) => ({ ...f, ...patch }));
  const onName = (name: string) =>
    setForm((f) => ({
      ...f,
      name,
      id: isNew ? slugify(name) || "new-rule" : f.id,
    }));

  const leaves: LeafNode[] = useMemo(() => {
    const direct = form.root.children.filter((c): c is LeafNode => c.kind === "leaf");
    return direct.length > 0 ? direct : findLeaves(form.root);
  }, [form.root]);

  function setLeaves(nextLeaves: LeafNode[], mode: "all" | "any" = form.root.kind) {
    update({
      root: {
        ...form.root,
        kind: mode,
        children: nextLeaves,
      },
    });
  }

  function updateLeaf(uid: string, patch: Partial<LeafNode>) {
    setLeaves(leaves.map((l) => (l.uid === uid ? { ...l, ...patch } : l)));
  }

  function addCondition() {
    setLeaves([...leaves, newLeaf()]);
  }

  function removeCondition(uid: string) {
    if (leaves.length <= 1) {
      setLeaves([{ ...leaves[0], field: "", op: "eq", value: "" }]);
      return;
    }
    setLeaves(leaves.filter((l) => l.uid !== uid));
  }

  function toggleRecipient(r: Recipient) {
    const has = form.recipients.some((x) => x.type === r.type && x.id === r.id);
    const nextRecipients = has
      ? form.recipients.filter((x) => !(x.type === r.type && x.id === r.id))
      : [...form.recipients, r];
    const firstTeam = nextRecipients.find((x) => x.type === "team")?.id ?? form.team;
    update({ recipients: nextRecipients, team: firstTeam });
  }

  async function handleAiSubmit() {
    const prompt = aiPrompt.trim();
    if (!prompt || aiBusy) return;
    setAiBusy(true);
    setAiError(null);
    setAiToolInfo(null);
    try {
      const sess = getSessionToken();
      const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
        Authorization: `Bearer ${bearer}`,
      };
      const personId = getPersonId();
      if (personId) headers["X-Person-Id"] = personId;

      const resp = await fetch(`${API_URL}/rules/draft`, {
        method: "POST",
        credentials: "include",
        headers,
        body: JSON.stringify({ prompt }),
      });
      if (!resp.ok) {
        const errBody = await resp.json().catch(() => null);
        setAiError(describeError(errBody, resp));
        return;
      }
      const outcome = (await resp.json()) as {
        tool_call?: string;
        draft?: Rule | null;
        target_rule_id?: string | null;
        message?: string | null;
        executed_tools?: string[];
      } & Partial<Rule>;

      const toolsUsed = outcome.executed_tools?.length
        ? outcome.executed_tools.join(" → ")
        : outcome.tool_call ?? "create_or_update_rule";
      setAiToolInfo(`Tool call: ${toolsUsed}`);

      if (outcome.tool_call === "delete_rule") {
        const targetId = outcome.target_rule_id ?? initial?.id;
        if (targetId) {
          const err = await send(() => api.DELETE("/rules/{rule_id}", { params: { path: { rule_id: targetId } } }));
          if (err) setAiError(`Failed to delete rule (${err}).`);
          else onDeleted(targetId);
        } else {
          setAiError("Model called delete_rule, but no target rule was specified.");
        }
        return;
      }

      const drafted: Rule | null = outcome.draft ?? (outcome.condition ? (outcome as Rule) : null);
      if (drafted) {
        const draftedForm = ruleToForm(drafted);
        setForm((prev) => ({
          ...draftedForm,
          id:
            outcome.tool_call === "select_rule" && drafted.id
              ? drafted.id
              : isNew
                ? slugify(draftedForm.name) || draftedForm.id || "new-rule"
                : prev.id,
          version: outcome.tool_call === "select_rule" ? (drafted.version ?? prev.version) : prev.version,
        }));
        setSubmitted(false);
      }
    } catch {
      setAiError("Failed to connect to local AI model (Ollama).");
    } finally {
      setAiBusy(false);
    }
  }

  async function save() {
    setSubmitted(true);
    setSaveError(null);
    if (errors.count > 0) return;
    setSaving(true);
    try {
      const body = rule;
      const res = isNew
        ? await api.POST("/rules", { body })
        : await api.PUT("/rules/{rule_id}", { params: { path: { rule_id: rule.id } }, body });
      if (res.error || !res.data) {
        setSaveError(apiMessage(res.error, res.response));
        return;
      }
      const saved: Rule = { ...rule, version: res.data.version ?? rule.version };
      const next = { ...form, version: saved.version ?? 0 };
      setForm(next);
      setBaseline(JSON.stringify(formToRule(next)));
      setSubmitted(false);
      onSaved(saved, isNew);
    } catch {
      setSaveError("Cannot connect to API.");
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!initial) return;
    setSaving(true);
    setSaveError(null);
    const err = await send(() => api.DELETE("/rules/{rule_id}", { params: { path: { rule_id: initial.id } } }));
    if (err) setSaveError(`Failed to delete rule (${err}).`);
    else onDeleted(initial.id);
    setSaving(false);
  }

  return (
    <div className="rules-editor">
      <datalist id={FIELD_LIST_ID}>
        {COMMON_FIELDS.map((f) => (
          <option key={f} value={f} />
        ))}
      </datalist>

      <header className="rules-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? "New rule" : form.name || initial?.name}
        </Heading>
      </header>

      {/* 1. AI Assistant */}
      <Card title="AI Assistant — create or edit a rule from natural language">
        <div className="rules-ai-box">
          <Field
            label="Describe the rule in plain English"
            hint='e.g. "When a trade on the rates desk is rejected due to LIMIT_BREACH, set severity to critical and notify Team Rates"'
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
                placeholder="Tell the AI how this rule should behave…"
              />
            )}
          </Field>
          <div className="rules-ai-actions">
            <Button
              appearance="primary"
              iconBefore={SparklesIcon}
              isDisabled={!aiPrompt.trim() || aiBusy}
              isLoading={aiBusy}
              onClick={() => void handleAiSubmit()}
            >
              Generate with AI
            </Button>
            {aiToolInfo && (
              <Text size="small" color="color.text.subtle">
                {aiToolInfo}
              </Text>
            )}
            {aiError && <ErrorMessage>{aiError}</ErrorMessage>}
          </div>
        </div>
      </Card>

      <fieldset className="rules-fieldset" disabled={saving}>
        {/* 2. Name & Severity */}
        <Card title="1. Name & severity">
          <div className="rules-grid">
            <Field label="Rule name" error={submitted ? errors.name : null}>
              {(f) => (
                <TextInput
                  {...f}
                  value={form.name}
                  onChange={(e) => onName(e.currentTarget.value)}
                  placeholder="e.g. Critical payment errors"
                  autoComplete="off"
                />
              )}
            </Field>
            <Field label="Alert severity">
              {(f) => (
                <Select
                  {...f}
                  value={form.severityOverride}
                  onChange={(v) => update({ severityOverride: v as RuleForm["severityOverride"] })}
                  options={[
                    { value: "", label: "Default (from alert)" },
                    ...SEVERITIES.map((x) => ({ value: x.value, label: x.label })),
                  ]}
                />
              )}
            </Field>
          </div>
        </Card>

        {/* 3. Conditions */}
        <Card title="2. When should this rule trigger?">
          <div className="rules-cond-mode">
            <Segmented
              label="Condition matching"
              value={form.root.kind}
              onChange={(mode) => setLeaves(leaves, mode)}
              options={[
                { value: "all", label: "All conditions must match (AND)" },
                { value: "any", label: "Any condition matches (OR)" },
              ]}
            />
          </div>

          <div className="rules-cond-list">
            {leaves.map((leaf, idx) => {
              const err = errors.leaves[leaf.uid];
              const fieldErr = err?.field && (submitted || err.kind === "format") ? err.field : undefined;
              const valueErr = err?.value && (submitted || err.kind === "format") ? err.value : undefined;
              return (
                <div key={leaf.uid} className="cb-leaf">
                  <Field label={idx === 0 ? "Event field" : "Field"} error={fieldErr}>
                    {(f) => (
                      <TextInput
                        {...f}
                        list={FIELD_LIST_ID}
                        value={leaf.field}
                        onChange={(e) => updateLeaf(leaf.uid, { field: e.currentTarget.value })}
                        placeholder="e.g. raw.labels.service"
                        autoComplete="off"
                      />
                    )}
                  </Field>
                  <Field label="Condition">
                    {(f) => (
                      <Select
                        {...f}
                        value={leaf.op}
                        onChange={(op) => updateLeaf(leaf.uid, { op: op as Op })}
                        options={OPS.map((o) => ({ value: o.op, label: o.label }))}
                      />
                    )}
                  </Field>
                  {leaf.op !== "exists" && (
                    <Field label="Value" error={valueErr}>
                      {(f) => (
                        <TextInput
                          {...f}
                          value={leaf.value}
                          onChange={(e) => updateLeaf(leaf.uid, { value: e.currentTarget.value })}
                          placeholder="e.g. payments-api"
                          autoComplete="off"
                        />
                      )}
                    </Field>
                  )}
                  <div className="cb-remove">
                    <Button appearance="subtle" onClick={() => removeCondition(leaf.uid)}>
                      Remove
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>

          <div className="cb-add">
            <Button onClick={addCondition}>+ Add condition</Button>
          </div>
        </Card>

        {/* 4. Recipients */}
        <Card title="3. Who should be notified?">
          <div className="rules-recipients">
            <fieldset className="rules-group">
              <legend>Teams</legend>
              {teams.length === 0 && <Text color="color.text.subtle">No teams</Text>}
              {teams.map((t) => (
                <Checkbox
                  key={t.id}
                  isChecked={form.recipients.some((r) => r.type === "team" && r.id === t.id)}
                  onChange={() => toggleRecipient({ type: "team", id: t.id })}
                  label={t.name}
                />
              ))}
            </fieldset>
            <fieldset className="rules-group">
              <legend>People</legend>
              {people.length === 0 && <Text color="color.text.subtle">No people</Text>}
              {people.map((p) => (
                <Checkbox
                  key={p.id}
                  isChecked={form.recipients.some((r) => r.type === "person" && r.id === p.id)}
                  onChange={() => toggleRecipient({ type: "person", id: p.id })}
                  label={p.name}
                />
              ))}
            </fieldset>
          </div>
          {submitted && errors.recipients && <ErrorMessage>{errors.recipients}</ErrorMessage>}
        </Card>

        {submitted && errors.count > 0 && (
          <SectionMessage appearance="error" title="Complete missing fields before saving">
            <Text as="p">Check the rule name, conditions, and make sure at least one recipient is selected.</Text>
          </SectionMessage>
        )}
        {saveError && <ErrorBox message={saveError} />}

        <div className="rules-savebar">
          <Button appearance="primary" onClick={() => void save()} isDisabled={saving || (!dirty && !isNew)} isLoading={saving}>
            {isNew ? "Create rule" : "Save rule"}
          </Button>
          {!isNew && (
            <ConfirmInline
              label="Delete rule"
              question={`Delete rule "${initial?.name}"?`}
              confirmLabel="Yes, delete"
              onConfirm={() => void remove()}
              busy={saving}
            />
          )}
        </div>
      </fieldset>
    </div>
  );
}
