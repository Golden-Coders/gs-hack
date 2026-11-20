// Edytor jednej reguły (tasks.md 15.1): zakładki „Kreator” i „YAML” nad jednym stanem formularza,
// podpowiedzi pól, odbiorcy, prezentacja, nadpisanie severity, test na zdarzeniu, zapis i usuwanie.
import { useEffect, useMemo, useRef, useState } from "react";
import { api, type Schemas } from "../../api/client";
import { paletteColor } from "../../components/colors";
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

function apiMessage(error: unknown): string {
  const e = error as { error?: string; detail?: string | null; line?: number | null } | undefined;
  if (!e?.error) return "Nie udało się zapisać reguły.";
  return `${e.error}${e.detail ? `: ${e.detail}` : ""}${e.line ? ` (linia ${e.line})` : ""}`;
}

function ColorSelect({ id, value, onChange, label, noneLabel }: { id: string; value: string; onChange: (v: string) => void; label: string; noneLabel: string }) {
  return (
    <span className="rules-color">
      <span
        className="rules-swatch"
        style={paletteColor(value) ? { background: paletteColor(value)!.bg, borderColor: paletteColor(value)!.fg } : undefined}
        aria-hidden="true"
      />
      <select id={id} value={value} onChange={(e) => onChange(e.target.value)} aria-label={label}>
        <option value="">{noneLabel}</option>
        {COLORS.map((c) => (
          <option key={c.value} value={c.value}>
            {c.label}
          </option>
        ))}
        {value && !COLORS.some((c) => c.value === value) && <option value={value}>{value} (nieznany)</option>}
      </select>
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
  const [savedNote, setSavedNote] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [schemaInput, setSchemaInput] = useState<string | null>(null);
  const [suggested, setSuggested] = useState<string[]>([]);
  const wizardTab = useRef<HTMLButtonElement>(null);
  const yamlTab = useRef<HTMLButtonElement>(null);

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
    (next === "yaml" ? yamlTab : wizardTab).current?.focus();
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
    setSavedNote(null);
    if ((tab === "yaml" && yamlError) || errors.count > 0) return;
    setSaving(true);
    try {
      const body = rule;
      const res = isNew
        ? await api.POST("/rules", { body })
        : await api.PUT("/rules/{rule_id}", { params: { path: { rule_id: rule.id } }, body });
      if (res.error || !res.data) {
        setSaveError(apiMessage(res.error));
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
      setSavedNote(`Zapisano regułę „${saved.name}” (wersja ${saved.version ?? "?"}). Obowiązuje dla kolejnych alertów.`);
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
    try {
      const res = await api.DELETE("/rules/{rule_id}", { params: { path: { rule_id: initial.id } } });
      if (!res.response.ok) {
        setSaveError(`Nie udało się usunąć reguły (HTTP ${res.response.status}).`);
        setConfirmDelete(false);
        return;
      }
      onDeleted(initial.id);
    } catch {
      setSaveError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  const testBlocked =
    tab === "yaml" && yamlError ? "Popraw błąd w YAML, aby przetestować regułę." : errors.count > 0 ? "Popraw błędy w regule (lista nad przyciskiem zapisu), aby ją przetestować." : undefined;
  const showErrorList = submitted && errorList.length > 0;

  function onTabKey(e: React.KeyboardEvent) {
    if (e.key === "ArrowRight" || e.key === "ArrowLeft" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      switchTab(tab === "wizard" ? "yaml" : "wizard");
    }
  }

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
        <h2>{isNew ? "Nowa reguła" : form.name || initial?.name}</h2>
        {!isNew && <span className="muted mono">{initial?.id}</span>}
        {!isNew && <span className="muted">wersja {form.version}</span>}
        {dirty && (
          <span className="rules-dirty" role="status">
            Niezapisane zmiany
          </span>
        )}
      </header>

      <div className="rules-tabs" role="tablist" aria-label="Sposób edycji reguły" onKeyDown={onTabKey}>
        <button
          ref={wizardTab}
          type="button"
          role="tab"
          id="rules-tab-wizard"
          aria-selected={tab === "wizard"}
          aria-controls="rules-panel-wizard"
          tabIndex={tab === "wizard" ? 0 : -1}
          onClick={() => switchTab("wizard")}
        >
          Kreator
        </button>
        <button
          ref={yamlTab}
          type="button"
          role="tab"
          id="rules-tab-yaml"
          aria-selected={tab === "yaml"}
          aria-controls="rules-panel-yaml"
          tabIndex={tab === "yaml" ? 0 : -1}
          onClick={() => switchTab("yaml")}
        >
          YAML <span className="muted">(zaawansowane)</span>
        </button>
      </div>

      <fieldset className="rules-fieldset" disabled={saving}>
        {tab === "wizard" ? (
          <div role="tabpanel" id="rules-panel-wizard" aria-labelledby="rules-tab-wizard" className="rules-panel">
            {wizardBanner && (
              <p className="rules-banner" role="status">
                YAML zawiera błąd, więc kreator pokazuje ostatnią poprawną wersję reguły. Wróć do zakładki YAML, aby go poprawić; zmiana w kreatorze zastąpi niepoprawny YAML.
              </p>
            )}

            <section className="rules-card" aria-labelledby="rc-basic">
              <h3 id="rc-basic">Podstawowe</h3>
              <div className="rules-grid">
                <div className="rules-row">
                  <label htmlFor="rule-name">Nazwa reguły</label>
                  <input
                    id="rule-name"
                    value={form.name}
                    onChange={(e) => onName(e.target.value)}
                    aria-invalid={submitted && errors.name ? true : undefined}
                    aria-describedby={submitted && errors.name ? "rule-name-err" : undefined}
                    autoComplete="off"
                  />
                  {submitted && errors.name && (
                    <span id="rule-name-err" className="rules-field-error">
                      {errors.name}
                    </span>
                  )}
                </div>
                <div className="rules-row">
                  <label htmlFor="rule-id">Identyfikator</label>
                  <input
                    id="rule-id"
                    className="mono"
                    value={form.id}
                    readOnly={!isNew}
                    onChange={(e) => {
                      setIdTouched(true);
                      update({ id: e.target.value });
                    }}
                    aria-invalid={submitted && errors.id ? true : undefined}
                    aria-describedby="rule-id-hint"
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span id="rule-id-hint" className={submitted && errors.id ? "rules-field-error" : "rules-field-hint"}>
                    {submitted && errors.id ? errors.id : isNew ? "Uzupełnia się z nazwy; po zapisie nie można go zmienić." : "Nie można zmienić."}
                  </span>
                </div>
                <div className="rules-row">
                  <label htmlFor="rule-team">Zespół właściciel</label>
                  <select id="rule-team" value={form.team} onChange={(e) => update({ team: e.target.value })}>
                    <option value="">(brak)</option>
                    {teams.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                    {form.team && !teams.some((t) => t.id === form.team) && <option value={form.team}>{form.team}</option>}
                  </select>
                </div>
                <div className="rules-row">
                  <label htmlFor="rule-sev">Nadpisz severity</label>
                  <select
                    id="rule-sev"
                    value={form.severityOverride}
                    onChange={(e) => update({ severityOverride: e.target.value as RuleForm["severityOverride"] })}
                  >
                    <option value="">bez zmian (wg mappingu)</option>
                    {SEVERITIES.map((s) => (
                      <option key={s.value} value={s.value}>
                        {s.label}
                      </option>
                    ))}
                  </select>
                  <span className="rules-field-hint">Dotyczy powiadomień tej reguły (kanały wg severity odbiorcy).</span>
                </div>
              </div>
              <label className="rules-check">
                <input type="checkbox" checked={form.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
                Reguła włączona
              </label>
            </section>

            <section className="rules-card" aria-labelledby="rc-cond">
              <h3 id="rc-cond">Kiedy reguła ma zadziałać</h3>
              <div className="rules-row rules-schema">
                <label htmlFor="rule-schema">Schemat zdarzeń (tylko do podpowiedzi pól)</label>
                <input
                  id="rule-schema"
                  className="mono"
                  list={SCHEMA_LIST_ID}
                  value={schemaInput ?? guessed}
                  onChange={(e) => setSchemaInput(e.target.value)}
                  placeholder="np. trade.rejected.v2"
                  autoComplete="off"
                  spellCheck={false}
                  aria-describedby="rule-schema-hint"
                />
                <span id="rule-schema-hint" className="rules-field-hint">
                  Pola z ostatnich zdarzeń tego schematu podpowiadają się w polu „Pole”.
                  {suggested.length > 0 && ` Znaleziono ${suggested.length}.`}
                </span>
              </div>
              <ConditionBuilder
                root={form.root}
                onChange={(root) => update({ root })}
                errors={errors}
                showRequired={submitted}
                fieldListId={FIELD_LIST_ID}
              />
            </section>

            <section className="rules-card" aria-labelledby="rc-rec">
              <h3 id="rc-rec">Kogo powiadomić</h3>
              <p className="muted rules-lead">Kanały (SMS, e-mail, telefon) wynikają z preferencji wybranych osób; zespół ma preferencje domyślne.</p>
              <div className="rules-recipients">
                <fieldset className="rules-group">
                  <legend>Osoby</legend>
                  {people.length === 0 && <span className="muted">brak osób w konfiguracji</span>}
                  {people.map((p) => (
                    <label key={p.id} className="rules-check">
                      <input
                        type="checkbox"
                        checked={form.recipients.some((r) => r.type === "person" && r.id === p.id)}
                        onChange={() => toggleRecipient({ type: "person", id: p.id })}
                      />
                      {p.name}
                    </label>
                  ))}
                </fieldset>
                <fieldset className="rules-group">
                  <legend>Zespoły</legend>
                  {teams.length === 0 && <span className="muted">brak zespołów w konfiguracji</span>}
                  {teams.map((t) => (
                    <label key={t.id} className="rules-check">
                      <input
                        type="checkbox"
                        checked={form.recipients.some((r) => r.type === "team" && r.id === t.id)}
                        onChange={() => toggleRecipient({ type: "team", id: t.id })}
                      />
                      {t.name}
                    </label>
                  ))}
                </fieldset>
                {unknownRecipients.length > 0 && (
                  <fieldset className="rules-group">
                    <legend>Nieznani w konfiguracji</legend>
                    {unknownRecipients.map((r) => (
                      <label key={`${r.type}:${r.id}`} className="rules-check">
                        <input type="checkbox" checked onChange={() => toggleRecipient(r)} />
                        {r.type === "team" ? "zespół" : "osoba"} {r.id}
                      </label>
                    ))}
                  </fieldset>
                )}
              </div>
              {submitted && errors.recipients && <span className="rules-field-error">{errors.recipients}</span>}
            </section>

            <section className="rules-card" aria-labelledby="rc-pres">
              <h3 id="rc-pres">Jak pokazać alert</h3>
              <div className="rules-grid">
                <div className="rules-row rules-wide">
                  <label htmlFor="rule-title">Tytuł (szablon)</label>
                  <input
                    id="rule-title"
                    className="mono"
                    value={form.title}
                    onChange={(e) => update({ title: e.target.value })}
                    aria-invalid={errors.title ? true : undefined}
                    aria-describedby="rule-title-hint"
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <span id="rule-title-hint" className={errors.title ? "rules-field-error" : "rules-field-hint"}>
                    {errors.title ?? "Pola wstawiaj jako {{raw.payload.desk}}; brakujące pole zostaje puste."}
                  </span>
                </div>
                <div className="rules-row rules-wide">
                  <label htmlFor="rule-body">Treść (szablon)</label>
                  <textarea
                    id="rule-body"
                    className="mono"
                    rows={3}
                    value={form.body}
                    onChange={(e) => update({ body: e.target.value })}
                    aria-invalid={errors.body ? true : undefined}
                    aria-describedby={errors.body ? "rule-body-err" : undefined}
                    spellCheck={false}
                  />
                  {errors.body && (
                    <span id="rule-body-err" className="rules-field-error">
                      {errors.body}
                    </span>
                  )}
                </div>
                <div className="rules-row">
                  <label htmlFor="rule-color">Kolor reguły</label>
                  <ColorSelect id="rule-color" value={form.color} onChange={(color) => update({ color })} label="Kolor reguły" noneLabel="bez koloru" />
                </div>
              </div>

              <h4 className="rules-subhead">Pokazywane pola</h4>
              {form.rows.length === 0 && <p className="muted rules-lead">Brak dodatkowych pól – alert pokaże tylko tytuł i treść.</p>}
              {form.rows.map((row, i) => (
                <div key={row.uid} className="rules-fieldrow">
                  <div className="rules-row">
                    <label htmlFor={`${row.uid}-path`}>Pole {i + 1}</label>
                    <input
                      id={`${row.uid}-path`}
                      data-focus={row.uid}
                      className="mono"
                      list={FIELD_LIST_ID}
                      value={row.path}
                      onChange={(e) => update({ rows: form.rows.map((r) => (r.uid === row.uid ? { ...r, path: e.target.value } : r)) })}
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </div>
                  <div className="rules-row">
                    <label htmlFor={`${row.uid}-color`}>Wyróżnienie</label>
                    <ColorSelect
                      id={`${row.uid}-color`}
                      value={row.color}
                      onChange={(color) => update({ rows: form.rows.map((r) => (r.uid === row.uid ? { ...r, color } : r)) })}
                      label={`Kolor wyróżnienia pola ${row.path || i + 1}`}
                      noneLabel="bez wyróżnienia"
                    />
                  </div>
                  <button
                    type="button"
                    className="cb-remove"
                    onClick={() => update({ rows: form.rows.filter((r) => r.uid !== row.uid) })}
                    aria-label={`Usuń pole ${row.path || i + 1}`}
                  >
                    Usuń
                  </button>
                </div>
              ))}
              <div className="rules-actions">
                <button type="button" onClick={() => update({ rows: [...form.rows, { uid: newUid(), path: "", color: "" }] })}>
                  + Dodaj pole
                </button>
              </div>
            </section>
          </div>
        ) : (
          <div role="tabpanel" id="rules-panel-yaml" aria-labelledby="rules-tab-yaml" className="rules-panel">
            <section className="rules-card" aria-labelledby="rc-yaml">
              <h3 id="rc-yaml">Reguła w YAML</h3>
              <p className="muted rules-lead">Zmiany są od razu widoczne w kreatorze. Niepoprawny YAML nie zmienia kreatora ani nie zostanie zapisany.</p>
              <div className="rules-row">
                <label htmlFor="rule-yaml">Treść YAML</label>
                <textarea
                  id="rule-yaml"
                  className="mono rules-yaml"
                  rows={24}
                  value={yamlText}
                  onChange={(e) => onYamlChange(e.target.value)}
                  spellCheck={false}
                  aria-invalid={yamlError ? true : undefined}
                  aria-describedby={yamlError ? "rule-yaml-err" : undefined}
                />
              </div>
              {yamlError && (
                <p id="rule-yaml-err" className="error-box" role="alert">
                  {yamlError.line ? `Linia ${yamlError.line}${yamlError.col ? `, kolumna ${yamlError.col}` : ""}: ` : ""}
                  {yamlError.message}
                </p>
              )}
            </section>
          </div>
        )}

        {showErrorList && (
          <div className="error-box rules-errorlist" role="alert">
            <strong>Popraw przed zapisem ({errorList.length}):</strong>
            <ul>
              {errorList.map((m, i) => (
                <li key={i}>{m}</li>
              ))}
            </ul>
          </div>
        )}
        {saveError && (
          <p className="error-box" role="alert">
            {saveError}
          </p>
        )}
        {savedNote && !dirty && (
          <p className="rules-saved" role="status">
            {savedNote}
          </p>
        )}

        <div className="rules-savebar">
          <button type="button" className="primary" onClick={save} disabled={saving || (!dirty && !isNew)}>
            {saving ? "Zapisywanie…" : isNew ? "Utwórz regułę" : "Zapisz zmiany"}
          </button>
          <button type="button" onClick={discard} disabled={saving || !dirty}>
            Odrzuć zmiany
          </button>
          {!isNew &&
            (confirmDelete ? (
              <span className="rules-confirm" role="alertdialog" aria-label="Potwierdzenie usunięcia">
                <span>Usunąć regułę „{initial?.name}”? Tego nie da się cofnąć.</span>
                <button type="button" className="danger" onClick={remove} disabled={saving}>
                  Tak, usuń
                </button>
                <button type="button" onClick={() => setConfirmDelete(false)}>
                  Anuluj
                </button>
              </span>
            ) : (
              <button type="button" className="danger" onClick={() => setConfirmDelete(true)}>
                Usuń regułę
              </button>
            ))}
        </div>
      </fieldset>

      <RuleTester rule={testBlocked ? null : rule} blockedReason={testBlocked} suggestedFields={suggested} />
    </div>
  );
}
