// Edytor jednej reguły (tasks.md 15.1): zakładki „Kreator” i „YAML” nad jednym stanem formularza,
// podpowiedzi pól, odbiorcy, prezentacja, nadpisanie severity, test na zdarzeniu, zapis i usuwanie.
import { useEffect, useMemo, useRef, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { useFlags } from "@atlaskit/flag";
import { ErrorMessage } from "@atlaskit/form";
import SectionMessage from "@atlaskit/section-message";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../../api/client";
import Lozenge from "@atlaskit/lozenge";
import { paletteColor } from "../../components/colors";
import { describeError, send } from "../../api/request";
import { Card, Checkbox, ConfirmInline, ErrorBox, Field, Select, Tabs, TextArea, TextInput } from "../../ui/primitives";
import ConditionBuilder from "./ConditionBuilder";
import RuleTester from "./RuleTester";
import {
  CANONICAL_FIELDS,
  COLORS,
  SEVERITIES,
  emptyForm,
  findLeaves,
  formToRule,
  guessSchema,
  newUid,
  parseRuleYaml,
  ruleToForm,
  ruleToYaml,
  slugify,
  validateForm,
  type Recipient,
  type Rule,
  type RuleForm,
  type YamlResult,
} from "./model";

type Person = Schemas["PersonDef"];
type Team = Schemas["TeamDef"];
type YamlFailure = Extract<YamlResult, { ok: false }>;

const FIELD_LIST_ID = "rule-field-options";
const SCHEMA_LIST_ID = "rule-schema-options";

interface Props {
  /** Edytowana reguła; null = nowa. */
  initial: Rule | null;
  people: Person[];
  teams: Team[];
  /** Znane schematy (z mappingów) – podpowiedzi do wyboru schematu dla podpowiedzi pól. */
  schemas: string[];
  onSaved: (rule: Rule, wasNew: boolean) => void;
  onDeleted: (id: string) => void;
  onDirtyChange: (dirty: boolean) => void;
}

/** Błąd zapisu z backendu; walidacja YAML po stronie serwera podaje też numer linii. */
function apiMessage(error: unknown, response: Response): string {
  const line = (error as { line?: number | null } | undefined)?.line;
  return `${describeError(error, response)}${line ? ` (linia ${line})` : ""}`;
}

function ColorSelect({ id, value, onChange, label, noneLabel }: { id: string; value: string; onChange: (v: string) => void; label: string; noneLabel: string }) {
  const color = paletteColor(value);
  const options = [
    { value: "", label: noneLabel },
    ...COLORS.map((c) => ({ value: c.value, label: c.label })),
    ...(value && !COLORS.some((c) => c.value === value) ? [{ value, label: `${value} (nieznany)` }] : []),
  ];
  return (
    <span className="rules-color">
      <span className="rules-swatch" style={color ? { background: color.bg, borderColor: color.fg } : undefined} aria-hidden="true" />
      <span className="rules-color-select">
        <Select id={id} value={value} onChange={onChange} options={options} aria-label={label} />
      </span>
    </span>
  );
}

export default function RuleEditor({ initial, people, teams, schemas, onSaved, onDeleted, onDirtyChange }: Props) {
  const isNew = initial === null;
  const [initialForm] = useState<RuleForm>(() => (initial ? ruleToForm(initial) : emptyForm()));
  const [form, setForm] = useState<RuleForm>(initialForm);
  const [baseline, setBaseline] = useState(() => JSON.stringify(formToRule(initialForm)));
  const [tab, setTab] = useState<"wizard" | "yaml">("wizard");
  const [yamlText, setYamlText] = useState("");
  const [yamlError, setYamlError] = useState<YamlFailure | null>(null);
  // Niepoprawny YAML, z którego wyszliśmy do kreatora: wracamy do niego, jeśli kreator nic nie zmienił.
  const brokenYaml = useRef<{ text: string; error: YamlFailure; snapshot: string } | null>(null);
  const [idTouched, setIdTouched] = useState(!isNew);
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const { showFlag } = useFlags();
  const [schemaInput, setSchemaInput] = useState<string | null>(null);
  const [suggested, setSuggested] = useState<string[]>([]);

  const rule = useMemo(() => formToRule(form), [form]);
  const snapshot = useMemo(() => JSON.stringify(rule), [rule]);
  const dirty = snapshot !== baseline || (tab === "yaml" && yamlError !== null);
  const errors = useMemo(() => validateForm(form, isNew), [form, isNew]);

  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  // Ostrzeżenie przy zamknięciu karty z niezapisanymi zmianami.
  useEffect(() => {
    if (!dirty) return;
    const handler = (e: BeforeUnloadEvent) => {
      e.preventDefault();
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [dirty]);

  // ---- podpowiedzi pól (GET /fields) dla schematu wpisanego lub wykrytego z warunku `schema = …`
  const guessed = guessSchema(form.root);
  const schema = (schemaInput ?? guessed).trim();
  useEffect(() => {
    if (!schema) {
      setSuggested([]);
      return;
    }
    let cancelled = false;
    const t = window.setTimeout(async () => {
      try {
        const { data } = await api.GET("/fields", { params: { query: { schema } } });
        if (!cancelled) setSuggested(data ?? []);
      } catch {
        if (!cancelled) setSuggested([]);
      }
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [schema]);
  const fieldOptions = useMemo(() => [...new Set([...CANONICAL_FIELDS, ...suggested])], [suggested]);

  // ---- zmiany formularza
  const update = (patch: Partial<RuleForm>) => setForm((f) => ({ ...f, ...patch }));
  const onName = (name: string) => setForm((f) => ({ ...f, name, id: idTouched ? f.id : slugify(name) }));

  function switchTab(next: "wizard" | "yaml") {
    if (next === tab) return;
    if (next === "yaml") {
      const b = brokenYaml.current;
      if (b && b.snapshot === snapshot) {
        setYamlText(b.text);
        setYamlError(b.error);
      } else {
        setYamlText(ruleToYaml(rule));
        setYamlError(null);
        brokenYaml.current = null;
      }
    } else if (yamlError) {
      brokenYaml.current = { text: yamlText, error: yamlError, snapshot };
    }
    setTab(next);
  }

  function onYamlChange(text: string) {
    setYamlText(text);
    const r = parseRuleYaml(text, form.version);
    if (!r.ok) {
      setYamlError(r);
      return;
    }
    if (!isNew && initial && r.rule.id !== initial.id) {
      setYamlError({ ok: false, message: `nie można zmienić id istniejącej reguły („${initial.id}”)` });
      return;
    }
    setYamlError(null);
    brokenYaml.current = null;
    if (isNew) setIdTouched(true);
    setForm(ruleToForm(r.rule));
  }

  // Zmiana w kreatorze zastępuje ewentualny niepoprawny YAML (kreator pamięta ostatni poprawny stan).
  const wizardBanner = tab === "wizard" && brokenYaml.current && brokenYaml.current.snapshot === snapshot;

  function toggleRecipient(r: Recipient) {
    const has = form.recipients.some((x) => x.type === r.type && x.id === r.id);
    update({ recipients: has ? form.recipients.filter((x) => !(x.type === r.type && x.id === r.id)) : [...form.recipients, r] });
  }

  const knownRecipients = new Set([...people.map((p) => `person:${p.id}`), ...teams.map((t) => `team:${t.id}`)]);
  const unknownRecipients = form.recipients.filter((r) => !knownRecipients.has(`${r.type}:${r.id}`));

  // ---- lista błędów do podsumowania nad przyciskiem zapisu
  const errorList = useMemo(() => {
    const out: string[] = [];
    if (errors.name) out.push(`Nazwa: ${errors.name}`);
    if (errors.id) out.push(`Identyfikator: ${errors.id}`);
    const leaves = findLeaves(form.root);
    for (const [uid, le] of Object.entries(errors.leaves)) {
      const leaf = leaves.find((l) => l.uid === uid);
      out.push(`Warunek „${leaf?.field || "bez pola"}”: ${le.field ?? le.value}`);
    }
    for (const msg of Object.values(errors.groups)) out.push(`Warunek: ${msg}`);
    if (errors.recipients) out.push(`Odbiorcy: ${errors.recipients}`);
    if (errors.title) out.push(`Tytuł: ${errors.title}`);
    if (errors.body) out.push(`Treść: ${errors.body}`);
    return out;
  }, [errors, form.root]);

  // ---- zapis / odrzucenie / usunięcie
  async function save() {
    setSubmitted(true);
    setSaveError(null);
    if ((tab === "yaml" && yamlError) || errors.count > 0) return;
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
      // Lokalny stan zostaje źródłem prawdy (serwer dopisuje tylko wersję).
      const saved: Rule = { ...rule, version: res.data.version ?? rule.version };
      const next = { ...form, version: saved.version ?? 0 };
      setForm(next);
      setBaseline(JSON.stringify(formToRule(next)));
      setYamlError(null);
      brokenYaml.current = null;
      setSubmitted(false);
      showFlag({ title: `Zapisano regułę „${saved.name}”`, description: `Wersja ${saved.version ?? "?"}. Obowiązuje dla kolejnych alertów.`, appearance: "success", isAutoDismiss: true });
      onSaved(saved, isNew);
    } catch {
      setSaveError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  function discard() {
    const f = initial ? ruleToForm(initial) : emptyForm();
    setForm(f);
    setBaseline(JSON.stringify(formToRule(f)));
    setYamlText(ruleToYaml(formToRule(f)));
    setYamlError(null);
    brokenYaml.current = null;
    setSubmitted(false);
    setSaveError(null);
    setIdTouched(!isNew);
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

  const testBlocked =
    tab === "yaml" && yamlError ? "Popraw błąd w YAML, aby przetestować regułę." : errors.count > 0 ? "Popraw błędy w regule (lista nad przyciskiem zapisu), aby ją przetestować." : undefined;
  const showErrorList = submitted && errorList.length > 0;

  return (
    <div className="rules-editor">
      <datalist id={FIELD_LIST_ID}>
        {fieldOptions.map((f) => (
          <option key={f} value={f} />
        ))}
      </datalist>
      <datalist id={SCHEMA_LIST_ID}>
        {schemas.map((s) => (
          <option key={s} value={s} />
        ))}
      </datalist>

      <header className="rules-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? "Nowa reguła" : form.name || initial?.name}
        </Heading>
        {!isNew && <Text color="color.text.subtle" as="span">{initial?.id}</Text>}
        {!isNew && <Text color="color.text.subtle" as="span">wersja {form.version}</Text>}
        {dirty && (
          <span role="status">
            <Lozenge appearance="moved">Niezapisane zmiany</Lozenge>
          </span>
        )}
      </header>

      <fieldset className="rules-fieldset" disabled={saving}>
        <Tabs
          label="Sposób edycji reguły"
          value={tab}
          onChange={switchTab}
          tabs={[
            { value: "wizard", label: "Kreator" },
            { value: "yaml", label: "YAML (zaawansowane)" },
          ]}
        >
        {tab === "wizard" ? (
          <div className="rules-panel">
            {wizardBanner && (
              <SectionMessage appearance="warning">
                YAML zawiera błąd, więc kreator pokazuje ostatnią poprawną wersję reguły. Wróć do zakładki YAML, aby go poprawić; zmiana w kreatorze zastąpi niepoprawny YAML.
              </SectionMessage>
            )}

            <Card title="Podstawowe">
              <div className="rules-grid">
                <Field label="Nazwa reguły" error={submitted ? errors.name : null}>
                  {(f) => <TextInput {...f} value={form.name} onChange={(e) => onName(e.currentTarget.value)} autoComplete="off" />}
                </Field>
                <Field
                  label="Identyfikator"
                  hint={submitted && errors.id ? undefined : isNew ? "Uzupełnia się z nazwy; po zapisie nie można go zmienić." : "Nie można zmienić."}
                  error={submitted ? errors.id : null}
                >
                  {(f) => (
                    <TextInput
                      {...f}
                      isMonospaced
                      value={form.id}
                      isReadOnly={!isNew}
                      onChange={(e) => {
                        setIdTouched(true);
                        update({ id: e.currentTarget.value });
                      }}
                      autoComplete="off"
                      spellCheck={false}
                    />
                  )}
                </Field>
                <Field label="Zespół właściciel">
                  {(f) => (
                    <Select
                      {...f}
                      value={form.team}
                      onChange={(team) => update({ team })}
                      options={[
                        { value: "", label: "(brak)" },
                        ...teams.map((t) => ({ value: t.id, label: t.name })),
                        ...(form.team && !teams.some((t) => t.id === form.team) ? [{ value: form.team, label: form.team }] : []),
                      ]}
                    />
                  )}
                </Field>
                <Field label="Nadpisz severity" hint="Dotyczy powiadomień tej reguły (kanały wg severity odbiorcy).">
                  {(f) => (
                    <Select
                      {...f}
                      value={form.severityOverride}
                      onChange={(v) => update({ severityOverride: v as RuleForm["severityOverride"] })}
                      options={[{ value: "", label: "bez zmian (wg mappingu)" }, ...SEVERITIES.map((x) => ({ value: x.value, label: x.label }))]}
                    />
                  )}
                </Field>
              </div>
              <div className="rules-check">
                <Checkbox isChecked={form.enabled} onChange={(e) => update({ enabled: e.target.checked })} label="Reguła włączona" />
              </div>
            </Card>

            <Card title="Kiedy reguła ma zadziałać">
              <div className="rules-schema">
                <Field
                  label="Schemat zdarzeń (tylko do podpowiedzi pól)"
                  hint={
                    <>
                      Pola z ostatnich zdarzeń tego schematu podpowiadają się w polu „Pole”.
                      {suggested.length > 0 && ` Znaleziono ${suggested.length}.`}
                    </>
                  }
                >
                  {(f) => (
                    <TextInput
                      {...f}
                      isMonospaced
                      list={SCHEMA_LIST_ID}
                      value={schemaInput ?? guessed}
                      onChange={(e) => setSchemaInput(e.currentTarget.value)}
                      placeholder="np. trade.rejected.v2"
                      autoComplete="off"
                      spellCheck={false}
                    />
                  )}
                </Field>
              </div>
              <ConditionBuilder
                root={form.root}
                onChange={(root) => update({ root })}
                errors={errors}
                showRequired={submitted}
                fieldListId={FIELD_LIST_ID}
              />
            </Card>

            <Card title="Kogo powiadomić">
              <Text as="p" color="color.text.subtle">Kanały (SMS, e-mail, telefon) wynikają z preferencji wybranych osób; zespół ma preferencje domyślne.</Text>
              <div className="rules-recipients">
                <fieldset className="rules-group">
                  <legend>Osoby</legend>
                  {people.length === 0 && <Text color="color.text.subtle">brak osób w konfiguracji</Text>}
                  {people.map((p) => (
                    <Checkbox
                      key={p.id}
                      isChecked={form.recipients.some((r) => r.type === "person" && r.id === p.id)}
                      onChange={() => toggleRecipient({ type: "person", id: p.id })}
                      label={p.name}
                    />
                  ))}
                </fieldset>
                <fieldset className="rules-group">
                  <legend>Zespoły</legend>
                  {teams.length === 0 && <Text color="color.text.subtle">brak zespołów w konfiguracji</Text>}
                  {teams.map((t) => (
                    <Checkbox
                      key={t.id}
                      isChecked={form.recipients.some((r) => r.type === "team" && r.id === t.id)}
                      onChange={() => toggleRecipient({ type: "team", id: t.id })}
                      label={t.name}
                    />
                  ))}
                </fieldset>
                {unknownRecipients.length > 0 && (
                  <fieldset className="rules-group">
                    <legend>Nieznani w konfiguracji</legend>
                    {unknownRecipients.map((r) => (
                      <Checkbox
                        key={`${r.type}:${r.id}`}
                        isChecked
                        onChange={() => toggleRecipient(r)}
                        label={`${r.type === "team" ? "zespół" : "osoba"} ${r.id}`}
                      />
                    ))}
                  </fieldset>
                )}
              </div>
              {submitted && errors.recipients && <ErrorMessage>{errors.recipients}</ErrorMessage>}
            </Card>

            <Card title="Jak pokazać alert">
              <div className="rules-grid">
                <div className="rules-wide">
                  <Field
                    label="Tytuł (szablon)"
                    hint={errors.title ? undefined : "Pola wstawiaj jako {{raw.payload.desk}}; brakujące pole zostaje puste."}
                    error={errors.title}
                  >
                    {(f) => (
                      <TextInput
                        {...f}
                        isMonospaced
                        value={form.title}
                        onChange={(e) => update({ title: e.currentTarget.value })}
                        autoComplete="off"
                        spellCheck={false}
                      />
                    )}
                  </Field>
                </div>
                <div className="rules-wide">
                  <Field label="Treść (szablon)" error={errors.body}>
                    {(f) => (
                      <TextArea {...f} isMonospaced minimumRows={3} value={form.body} onChange={(e) => update({ body: e.target.value })} spellCheck={false} />
                    )}
                  </Field>
                </div>
                <Field label="Kolor reguły">
                  {(f) => <ColorSelect id={f.id} value={form.color} onChange={(color) => update({ color })} label="Kolor reguły" noneLabel="bez koloru" />}
                </Field>
              </div>

              <Heading size="xsmall" as="h4">Pokazywane pola</Heading>
              {form.rows.length === 0 && <Text as="p" color="color.text.subtle">Brak dodatkowych pól – alert pokaże tylko tytuł i treść.</Text>}
              {form.rows.map((row, i) => (
                <div key={row.uid} className="rules-fieldrow">
                  <Field label={`Pole ${i + 1}`}>
                    {(f) => (
                      <TextInput
                        {...f}
                        isMonospaced
                        list={FIELD_LIST_ID}
                        value={row.path}
                        onChange={(e) => update({ rows: form.rows.map((r) => (r.uid === row.uid ? { ...r, path: e.currentTarget.value } : r)) })}
                        autoComplete="off"
                        spellCheck={false}
                      />
                    )}
                  </Field>
                  <Field label="Wyróżnienie">
                    {(f) => (
                      <ColorSelect
                        id={f.id}
                        value={row.color}
                        onChange={(color) => update({ rows: form.rows.map((r) => (r.uid === row.uid ? { ...r, color } : r)) })}
                        label={`Kolor wyróżnienia pola ${row.path || i + 1}`}
                        noneLabel="bez wyróżnienia"
                      />
                    )}
                  </Field>
                  <div className="rules-fieldrow-remove">
                    <Button onClick={() => update({ rows: form.rows.filter((r) => r.uid !== row.uid) })} aria-label={`Usuń pole ${row.path || i + 1}`}>
                      Usuń
                    </Button>
                  </div>
                </div>
              ))}
              <div className="rules-actions">
                <Button onClick={() => update({ rows: [...form.rows, { uid: newUid(), path: "", color: "" }] })}>Dodaj pole</Button>
              </div>
            </Card>
          </div>
        ) : (
          <div className="rules-panel">
            <Card title="Reguła w YAML">
              <Text as="p" color="color.text.subtle">Zmiany są od razu widoczne w kreatorze. Niepoprawny YAML nie zmienia kreatora ani nie zostanie zapisany.</Text>
              <Field label="Treść YAML">
                {(f) => (
                  <TextArea
                    {...f}
                    isMonospaced
                    minimumRows={24}
                    value={yamlText}
                    onChange={(e) => onYamlChange(e.target.value)}
                    spellCheck={false}
                    aria-invalid={yamlError ? true : undefined}
                    aria-describedby={yamlError ? "rule-yaml-err" : undefined}
                  />
                )}
              </Field>
              {yamlError && (
                <div id="rule-yaml-err">
                  <ErrorBox
                    message={`${yamlError.line ? `Linia ${yamlError.line}${yamlError.col ? `, kolumna ${yamlError.col}` : ""}: ` : ""}${yamlError.message}`}
                  />
                </div>
              )}
            </Card>
          </div>
        )}
        </Tabs>

        {showErrorList && (
          <div role="alert">
            <SectionMessage appearance="error" headingLevel="h3" title={`Popraw przed zapisem (${errorList.length})`}>
              <ul className="rules-errorlist">
                {errorList.map((m, i) => (
                  <li key={i}>{m}</li>
                ))}
              </ul>
            </SectionMessage>
          </div>
        )}
        {saveError && <ErrorBox message={saveError} />}

        <div className="rules-savebar">
          <Button appearance="primary" onClick={save} isDisabled={saving || (!dirty && !isNew)} isLoading={saving}>
            {isNew ? "Utwórz regułę" : "Zapisz zmiany"}
          </Button>
          <Button onClick={discard} isDisabled={saving || !dirty}>
            Odrzuć zmiany
          </Button>
          {!isNew && (
            <ConfirmInline
              label="Usuń regułę"
              question={`Usunąć regułę „${initial?.name}”? Tego nie da się cofnąć.`}
              confirmLabel="Tak, usuń"
              onConfirm={() => void remove()}
              busy={saving}
            />
          )}
        </div>
      </fieldset>

      <RuleTester rule={testBlocked ? null : rule} blockedReason={testBlocked} suggestedFields={suggested} />
    </div>
  );
}
