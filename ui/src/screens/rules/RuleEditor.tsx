// Przejrzysty, maksymalnie prosty edytor reguły:
// 1. Panel komunikacji z AI (gotowy do podpięcia pod backend)
// 2. Nazwa + priorytet
// 3. Płaska, czytelna lista warunków (Pole -> Operator -> Wartość)
// 4. Wybór odbiorców (Zespoły / Osoby)
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
  return `${describeError(error, response)}${line ? ` (linia ${line})` : ""}`;
}

export default function RuleEditor({ initial, people, teams, onSaved, onDeleted, onDirtyChange }: Props) {
  const isNew = initial === null;
  const [initialForm] = useState<RuleForm>(() => (initial ? ruleToForm(initial) : emptyForm()));
  const [form, setForm] = useState<RuleForm>(initialForm);
  const [baseline, setBaseline] = useState(() => JSON.stringify(formToRule(initialForm)));
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Panel AI (połączony z lokalnym modelem Ollama i tool-callami przez POST /rules/draft)
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
      id: isNew ? slugify(name) || "nowa-regula" : f.id,
    }));

  // Płaska lista warunków z głównej grupy (prosto i czytelnie)
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
          if (err) setAiError(`Nie udało się usunąć reguły (${err}).`);
          else onDeleted(targetId);
        } else {
          setAiError("Model wywołał delete_rule, ale nie wskazano reguły do usunięcia.");
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
                ? slugify(draftedForm.name) || draftedForm.id || "nowa-regula"
                : prev.id,
          version: outcome.tool_call === "select_rule" ? (drafted.version ?? prev.version) : prev.version,
        }));
        setSubmitted(false);
      }
    } catch {
      setAiError("Nie udało się połączyć z lokalnym modelem AI (Ollama).");
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
      setSaveError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!initial) return;
    setSaving(true);
    setSaveError(null);
    const err = await send(() => api.DELETE("/rules/{rule_id}", { params: { path: { rule_id: initial.id } } }));
    if (err) setSaveError(`Nie udało się usunąć reguły (${err}).`);
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
          {isNew ? "Nowa reguła" : form.name || initial?.name}
        </Heading>
      </header>

      {/* 1. Panel AI */}
      <Card title="Asystent AI — stwórz lub zmień regułę opisem">
        <div className="rules-ai-box">
          <Field
            label="Opisz regułę własnymi słowami"
            hint="Np. „Gdy transakcja na desku rates zostanie odrzucona z powodu LIMIT_BREACH, ustaw priorytet krytyczny i powiadom zespół Rates”"
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
                placeholder="Napisz dla AI, jak ma działać ta reguła…"
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
              Wygeneruj z AI
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
        {/* 2. Podstawowe informacje */}
        <Card title="1. Nazwa i priorytet">
          <div className="rules-grid">
            <Field label="Nazwa reguły" error={submitted ? errors.name : null}>
              {(f) => (
                <TextInput
                  {...f}
                  value={form.name}
                  onChange={(e) => onName(e.currentTarget.value)}
                  placeholder="np. Krytyczne błędy płatności"
                  autoComplete="off"
                />
              )}
            </Field>
            <Field label="Priorytet alertu">
              {(f) => (
                <Select
                  {...f}
                  value={form.severityOverride}
                  onChange={(v) => update({ severityOverride: v as RuleForm["severityOverride"] })}
                  options={[
                    { value: "", label: "Domyślny (z alertu)" },
                    ...SEVERITIES.map((x) => ({ value: x.value, label: x.label })),
                  ]}
                />
              )}
            </Field>
          </div>
        </Card>

        {/* 3. Warunki */}
        <Card title="2. Kiedy reguła ma zadziałać?">
          <div className="rules-cond-mode">
            <Segmented
              label="Łączenie warunków"
              value={form.root.kind}
              onChange={(mode) => setLeaves(leaves, mode)}
              options={[
                { value: "all", label: "Wszystkie warunki muszą pasować (I)" },
                { value: "any", label: "Wystarczy dowolny warunek (LUB)" },
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
                  <Field label={idx === 0 ? "Pole zdarzenia" : "Pole"} error={fieldErr}>
                    {(f) => (
                      <TextInput
                        {...f}
                        list={FIELD_LIST_ID}
                        value={leaf.field}
                        onChange={(e) => updateLeaf(leaf.uid, { field: e.currentTarget.value })}
                        placeholder="np. raw.labels.service"
                        autoComplete="off"
                      />
                    )}
                  </Field>
                  <Field label={idx === 0 ? "Warunek" : "Warunek"}>
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
                    <Field label={idx === 0 ? "Wartość" : "Wartość"} error={valueErr}>
                      {(f) => (
                        <TextInput
                          {...f}
                          value={leaf.value}
                          onChange={(e) => updateLeaf(leaf.uid, { value: e.currentTarget.value })}
                          placeholder="np. payments-api"
                          autoComplete="off"
                        />
                      )}
                    </Field>
                  )}
                  <div className="cb-remove">
                    <Button appearance="subtle" onClick={() => removeCondition(leaf.uid)}>
                      Usuń
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>

          <div className="cb-add">
            <Button onClick={addCondition}>+ Dodaj kolejny warunek</Button>
          </div>
        </Card>

        {/* 4. Odbiorcy */}
        <Card title="3. Kogo powiadomić?">
          <div className="rules-recipients">
            <fieldset className="rules-group">
              <legend>Zespoły</legend>
              {teams.length === 0 && <Text color="color.text.subtle">Brak zespołów</Text>}
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
              <legend>Osoby</legend>
              {people.length === 0 && <Text color="color.text.subtle">Brak osób</Text>}
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
          <SectionMessage appearance="error" title="Uzupełnij brakujące pola przed zapisem">
            <Text as="p">Sprawdź nazwę reguły, warunki oraz czy wybrano przynajmniej jednego odbiorcę.</Text>
          </SectionMessage>
        )}
        {saveError && <ErrorBox message={saveError} />}

        <div className="rules-savebar">
          <Button appearance="primary" onClick={() => void save()} isDisabled={saving || (!dirty && !isNew)} isLoading={saving}>
            {isNew ? "Utwórz regułę" : "Zapisz regułę"}
          </Button>
          {!isNew && (
            <ConfirmInline
              label="Usuń regułę"
              question={`Usunąć regułę „${initial?.name}”?`}
              confirmLabel="Tak, usuń"
              onConfirm={() => void remove()}
              busy={saving}
            />
          )}
        </div>
      </fieldset>
    </div>
  );
}
