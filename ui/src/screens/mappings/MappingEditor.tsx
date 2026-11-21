// Edytor mappingu krok po kroku: schemat -> zaznacz pola w payloadzie -> severity/fingerprint/resolved -> podgląd -> zapis.
// Kliknięcie w drzewie payloadu dodaje pole do aktualnie wybranego celu (pola, tytuł, fingerprint, severity, resolved).
import { useMemo, useState, type ComponentProps } from "react";
import Button from "@atlaskit/button/new";
import { ErrorMessage } from "@atlaskit/form";
import { Text } from "@atlaskit/primitives/compiled";
import SectionMessage from "@atlaskit/section-message";
import { RadioGroup } from "@atlaskit/radio";
import { api } from "../../api/client";
import { describeError } from "../../api/request";
import ConditionView from "../../components/ConditionView";
import PayloadTree from "../../components/PayloadTree";
import { Card, Checkbox, Empty, ErrorBox, Field, PageHeader, Segmented, Select, TextArea, TextInput } from "../../ui/primitives";
import PreviewPanel from "./PreviewPanel";
import {
  OP_LABEL,
  SEVERITIES,
  SIMPLE_OPS,
  coerceValue,
  draftFromMapping,
  isSimpleLeaf,
  mappingFromDraft,
  nextId,
  validate,
  valueToText,
  type Cond,
  type Draft,
  type Mapping,
  type Severity,
  type SeverityDraft,
  type SimpleOp,
} from "./draft";
import {
  defaultFieldName,
  parseConditionField,
  parseJsonPath,
  pathKey,
  toConditionField,
  toJsonPath,
  type PathSeg,
} from "../../lib/paths";
import { parseSample } from "./sample";

type Mode = "fields" | "title" | "fingerprint" | "severity" | "resolved";

const MODES: { id: Mode; label: string; hint: string }[] = [
  { id: "fields", label: "Pola", hint: "Klik w pole dodaje je do pól zdarzenia (ponowny klik usuwa)." },
  { id: "title", label: "Tytuł", hint: "Klik w pole ustawia je jako tytuł alertu." },
  { id: "fingerprint", label: "Fingerprint", hint: "Klik w pole dodaje je do fingerprintu: zdarzenia o tych samych wartościach łączą się w jeden alert." },
  { id: "severity", label: "Severity", hint: "Klik w pole wskazuje, skąd brać severity (przy trybie „Warunki” dodaje regułę)." },
  { id: "resolved", label: "Resolved", hint: "Klik w pole ustawia warunek „pole = wartość z przykładu” oznaczający rozwiązanie problemu." },
];

const MODE_LABEL: Record<Mode, string> = {
  fields: "pole",
  title: "tytuł",
  fingerprint: "fingerprint",
  severity: "severity",
  resolved: "resolved",
};

const SEVERITY_GUESS: Record<string, Severity> = { critical: "critical", crit: "critical", error: "error", warning: "warning", warn: "warning", info: "info" };

type ApiError = { error?: string; detail?: string | null; line?: number | null } | undefined;

/** Komunikat serwera z numerem linii (błędy parsowania mappingu), gdy backend go podał. */
function describeSaveError(e: ApiError, response: Response): string {
  const msg = describeError(e, response);
  return e?.line ? `${msg} (linia ${e.line})` : msg;
}

const SEVERITY_OPTIONS = SEVERITIES.map((v) => ({ value: v, label: v }));

function SeveritySelect({
  value,
  onChange,
  ...rest
}: { value: Severity; onChange: (s: Severity) => void } & Omit<ComponentProps<typeof Select>, "value" | "onChange" | "options">) {
  return <Select {...rest} options={SEVERITY_OPTIONS} value={value} onChange={(v) => onChange(v as Severity)} />;
}

/** Wiersz warunku-liścia: pole, operator, wartość. */
function LeafEditor({ cond, onChange, label }: { cond: Cond & { field: string }; onChange: (c: Cond) => void; label: string }) {
  const op = (cond.op ?? "eq") as SimpleOp;
  return (
    <div className="mp-leaf">
      <Field label="Pole payloadu">
        {(f) => (
          <TextInput
            {...f}
            className="mono"
            value={cond.field}
            onChange={(e) => onChange({ ...cond, field: e.currentTarget.value })}
            aria-label={`${label}: pole`}
            placeholder="raw.status"
          />
        )}
      </Field>
      <Field label="Operator">
        {(f) => (
          <Select
            {...f}
            value={op}
            options={SIMPLE_OPS.map((o) => ({ value: o, label: OP_LABEL[o] }))}
            onChange={(v) => onChange({ ...cond, op: v as SimpleOp })}
            aria-label={`${label}: operator`}
          />
        )}
      </Field>
      {op !== "exists" && (
        <Field label="Wartość">
          {(f) => (
            <TextInput
              {...f}
              value={valueToText(cond.value)}
              onChange={(e) => onChange({ ...cond, value: coerceValue(e.currentTarget.value, cond.value, op) })}
              aria-label={`${label}: wartość`}
            />
          )}
        </Field>
      )}
    </div>
  );
}

function Section({
  step,
  title,
  mode,
  active,
  onPick,
  children,
}: {
  step?: string;
  title: string;
  mode?: Mode;
  active?: Mode;
  onPick?: (m: Mode) => void;
  children: React.ReactNode;
}) {
  return (
    <Card
      title={
        <>
          {step && <span className="mp-step">{step}</span>}
          {title}
        </>
      }
      actions={
        mode &&
        onPick && (
          <Button spacing="compact" isSelected={active === mode} aria-pressed={active === mode} onClick={() => onPick(mode)}>
            {active === mode ? "Wybierasz z drzewa" : "Wybierz z drzewa"}
          </Button>
        )
      }
    >
      {children}
    </Card>
  );
}

export default function MappingEditor({
  initial,
  sample,
  eventCount,
  isNew,
  onSaved,
  onClose,
}: {
  initial: Mapping;
  /** Przykładowy payload (obiekt JSON albo tekst YAML/JSON); undefined = brak, trzeba wkleić. */
  sample: unknown;
  eventCount?: number;
  isNew: boolean;
  onSaved: (m: Mapping) => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<Draft>(() => draftFromMapping(initial));
  const [mode, setMode] = useState<Mode>("fields");
  const [sampleText, setSampleText] = useState(() =>
    sample === undefined ? "" : typeof sample === "string" ? sample : JSON.stringify(sample, null, 2),
  );
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState(false);

  const parsed = useMemo(() => (sampleText.trim() ? parseSample(sampleText) : null), [sampleText]);
  const payload = parsed && parsed.format !== "invalid" ? parsed.value : undefined;
  const problems = validate(draft);
  const mapping = useMemo(() => mappingFromDraft(draft), [draft]);

  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const setSeverity = (severity: SeverityDraft) => setDraft((d) => ({ ...d, severity }));

  // ---- oznaczenia w drzewie
  const marks = useMemo(() => {
    const m = new Map<string, string[]>();
    const add = (segs: PathSeg[] | null, label: string) => {
      if (!segs) return;
      const k = pathKey(segs);
      m.set(k, [...(m.get(k) ?? []), label]);
    };
    draft.fields.forEach((f) => add(parseJsonPath(f.path), f.name ? `pole ${f.name}` : "pole"));
    if (draft.title.trim().startsWith("$")) add(parseJsonPath(draft.title), "tytuł");
    draft.fingerprint.forEach((p) => add(parseJsonPath(p), "fingerprint"));
    const s = draft.severity;
    if (s.type === "path") add(parseJsonPath(s.path), "severity");
    if (s.type === "rules") s.rules.forEach((r) => add(r.when.field ? parseConditionField(r.when.field) : null, "severity"));
    if (draft.resolved?.field) add(parseConditionField(draft.resolved.field), "resolved");
    return m;
  }, [draft]);

  // ---- klik w drzewie
  function pick(segs: PathSeg[], value: unknown) {
    const jp = toJsonPath(segs);
    if (mode === "fields") {
      setDraft((d) => {
        const existing = d.fields.find((f) => f.path === jp);
        if (existing) return { ...d, fields: d.fields.filter((f) => f !== existing) };
        let name = defaultFieldName(segs);
        const taken = new Set(d.fields.map((f) => f.name));
        for (let i = 2; taken.has(name); i++) name = `${defaultFieldName(segs)}${i}`;
        return { ...d, fields: [...d.fields, { id: nextId(), name, path: jp }] };
      });
    } else if (mode === "title") {
      setDraft((d) => ({ ...d, title: d.title === jp ? "" : jp }));
    } else if (mode === "fingerprint") {
      setDraft((d) => ({
        ...d,
        fingerprint: d.fingerprint.includes(jp) ? d.fingerprint.filter((p) => p !== jp) : [...d.fingerprint, jp],
      }));
    } else if (mode === "severity") {
      setDraft((d) => {
        const s = d.severity;
        if (s.type === "rules") {
          const numeric = typeof value === "number";
          const rule = {
            id: nextId(),
            when: { field: toConditionField(segs), op: numeric ? "gt" : "eq", value } as Cond,
            value: "critical" as Severity,
          };
          return { ...d, severity: { ...s, rules: [...s.rules, rule] } };
        }
        const text = typeof value === "string" ? value : null;
        const guess = text ? SEVERITY_GUESS[text.toLowerCase()] : undefined;
        const map = text && !guess ? [{ id: nextId(), key: text, value: "warning" as Severity }] : [];
        const def = s.type === "path" ? s.default : "warning";
        return { ...d, severity: { type: "path", path: jp, map: s.type === "path" && s.path === jp ? s.map : map, default: def } };
      });
    } else {
      setDraft((d) => ({ ...d, resolved: { field: toConditionField(segs), op: "eq", value } }));
    }
  }

  async function save() {
    setSubmitted(true);
    setSaveError(null);
    if (problems.any) return;
    setSaving(true);
    try {
      const { data, error, response } = await api.PUT("/mappings/{schema}", {
        params: { path: { schema: mapping.schema } },
        body: mapping,
      });
      // nazwa schematu z żądania (nie z odpowiedzi), wersja nadana przez serwer
      if (data) onSaved({ ...data, schema: mapping.schema });
      else setSaveError(describeSaveError(error as ApiError, response));
    } catch (e) {
      setSaveError(`Nie udało się połączyć z serwerem: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSaving(false);
    }
  }

  const blocked = !payload
    ? "Podgląd pojawi się po wskazaniu przykładowego payloadu."
    : problems.any
      ? "Podgląd pojawi się po poprawieniu błędów w formularzu."
      : undefined;
  const showErr = (msg?: string) => (msg ? <div role="alert"><ErrorMessage>{msg}</ErrorMessage></div> : null);
  const modeInfo = MODES.find((m) => m.id === mode)!;
  const sev = draft.severity;

  return (
    <div className="mp-editor">
      <PageHeader
        actions={
          <Button onClick={onClose} isDisabled={saving}>
            Zamknij edytor
          </Button>
        }
      >
        {isNew ? "Nowy mapping" : "Edycja mappingu"}: <span className="mono">{draft.schema || "…"}</span>
      </PageHeader>
      <Text as="p" color="color.text.subtle">
        {isNew
          ? "Schemat wybrany (krok 1 z 5). Zaznacz pola w payloadzie, ustaw severity i fingerprint, sprawdź podgląd i zapisz."
          : `Wersja ${draft.version ?? "?"}. Zmiana działa na kolejne zdarzenia bez restartu.`}
        {eventCount !== undefined && ` Czekających zdarzeń UNMAPPED: ${eventCount} – po zapisie zostaną przetworzone ponownie.`}
      </Text>

      <div className="mp-grid">
        {/* ----------------------------------------------------------- lewa kolumna: payload */}
        <div className="mp-left">
          <Section step="Krok 2" title="Zaznacz pola w payloadzie">
            <Segmented
              label="Cel kliknięcia w drzewie"
              options={MODES.map((m) => ({ value: m.id, label: m.label }))}
              value={mode}
              onChange={setMode}
            />
            <div aria-live="polite">
              <Text as="p" color="color.text.subtle">
                <Text weight="bold">Klik dodaje do: {MODE_LABEL[mode]}.</Text> {modeInfo.hint}
            
              </Text>
            </div>
            {payload !== undefined ? (
              <div className="mp-tree-box">
                <PayloadTree value={payload} onSelect={pick} marks={marks} actionLabel={`Dodaj do: ${MODE_LABEL[mode]}`} />
              </div>
            ) : (
              <Empty>Brak przykładowego payloadu – wklej go poniżej (JSON lub YAML).</Empty>
            )}
            <details className="mp-sample" open={payload === undefined}>
              <summary>
                Przykładowy payload{parsed && parsed.format !== "invalid" ? ` (${parsed.format.toUpperCase()})` : ""} – zmień
              </summary>
              <Field label="Wklej zdarzenie (JSON lub YAML)">
                {(f) => (
                  <TextArea
                    {...f}
                    className="mono"
                    rows={8}
                    value={sampleText}
                    onChange={(e) => setSampleText(e.currentTarget.value)}
                    spellCheck={false}
                  />
                )}
              </Field>
              {parsed?.format === "invalid" && <div role="alert"><ErrorMessage>Nie udało się odczytać payloadu: {parsed.error}</ErrorMessage></div>}
            </details>
          </Section>
        </div>

        {/* ----------------------------------------------------------- prawa kolumna: ustawienia */}
        <div className="mp-right">
          <Section step="Krok 3" title="Pola kanoniczne i tytuł" mode="fields" active={mode} onPick={setMode}>
            {draft.fields.length === 0 ? (
              <Text as="p" color="color.text.subtle">Brak pól. Wybierz „Pola” nad drzewem i kliknij pole payloadu.</Text>
            ) : (
              <ul className="mp-rows">
                {draft.fields.map((f) => (
                  <li key={f.id}>
                    <div className="mp-row">
                      <Field label="Nazwa pola">
                        {(a) => (
                          <TextInput
                            {...a}
                            value={f.name}
                            onChange={(e) => set({ fields: draft.fields.map((x) => (x.id === f.id ? { ...x, name: e.currentTarget.value } : x)) })}
                          />
                        )}
                      </Field>
                      <div className="mp-grow">
                        <Field label="Ścieżka">
                          {(a) => (
                            <TextInput
                              {...a}
                              className="mono"
                              value={f.path}
                              onChange={(e) => set({ fields: draft.fields.map((x) => (x.id === f.id ? { ...x, path: e.currentTarget.value } : x)) })}
                            />
                          )}
                        </Field>
                      </div>
                      <Button onClick={() => set({ fields: draft.fields.filter((x) => x.id !== f.id) })} aria-label={`Usuń pole ${f.name}`}>Usuń</Button>
                    </div>
                    {showErr(problems.fields[f.id])}
                  </li>
                ))}
              </ul>
            )}
            <Field label={<>Tytuł alertu (ścieżka lub szablon, np. {"{{raw.error.message}}"})</>} error={problems.title}>
              {(a) => (
                <div className="mp-inline">
                  <TextInput
                    {...a}
                    className="mono mp-grow"
                    value={draft.title}
                    onChange={(e) => set({ title: e.currentTarget.value })}
                    placeholder="$.error.message"
                  />
                  <Button spacing="compact" isSelected={mode === "title"} aria-pressed={mode === "title"} onClick={() => setMode("title")}>
                    Wybierz z drzewa
                  </Button>
                </div>
              )}
            </Field>
            {!draft.title.trim() && <Text as="p" size="small" color="color.text.subtle">Bez tytułu alert dostanie nazwę schematu.</Text>}
          </Section>

          <Section title="Severity" mode="severity" active={mode} onPick={setMode}>
            <Text as="p" weight="bold" id="sev-type-label">
              Skąd brać severity
            </Text>
            <RadioGroup
              labelId="sev-type-label"
              name="sev-type"
              value={sev.type}
              options={[
                { name: "sev-type", value: "const", label: "Stała wartość" },
                { name: "sev-type", value: "path", label: "Z pola payloadu" },
                { name: "sev-type", value: "rules", label: "Z warunków" },
              ]}
              onChange={(e) => {
                const t = e.currentTarget.value as SeverityDraft["type"];
                if (t === sev.type) return;
                const def = sev.type === "const" ? sev.value : sev.default;
                setSeverity(
                  t === "const"
                    ? { type: "const", value: def }
                    : t === "path"
                      ? { type: "path", path: "", map: [], default: def }
                      : { type: "rules", rules: [], default: def },
                );
              }}
            />
            {sev.type === "const" && (
              <Field label="Severity wszystkich zdarzeń">
                {(f) => <SeveritySelect {...f} value={sev.value} onChange={(value) => setSeverity({ type: "const", value })} aria-label="Severity" />}
              </Field>
            )}
            {sev.type === "path" && (
              <>
                <Field label="Pole z severity">
                  {(f) => (
                    <TextInput
                      {...f}
                      className="mono"
                      value={sev.path}
                      onChange={(e) => setSeverity({ ...sev, path: e.currentTarget.value })}
                      placeholder="$.severity"
                    />
                  )}
                </Field>
                <Text as="p" size="small" color="color.text.subtle">
                  Nazwy critical, error, warning/warn, info są rozpoznawane same. Inne wartości źródła przypisz poniżej.
                </Text>
                {sev.map.length > 0 && (
                  <ul className="mp-rows">
                    {sev.map.map((r) => (
                      <li key={r.id} className="mp-row">
                        <div className="mp-grow">
                          <Field label="Wartość w źródle">
                            {(f) => (
                              <TextInput
                                {...f}
                                value={r.key}
                                onChange={(e) => setSeverity({ ...sev, map: sev.map.map((x) => (x.id === r.id ? { ...x, key: e.currentTarget.value } : x)) })}
                              />
                            )}
                          </Field>
                        </div>
                        <Field label="Severity">
                          {(f) => (
                            <SeveritySelect
                              {...f}
                              value={r.value}
                              aria-label={`Severity dla ${r.key || "wartości"}`}
                              onChange={(value) => setSeverity({ ...sev, map: sev.map.map((x) => (x.id === r.id ? { ...x, value } : x)) })}
                            />
                          )}
                        </Field>
                        <Button onClick={() => setSeverity({ ...sev, map: sev.map.filter((x) => x.id !== r.id) })} aria-label={`Usuń przypisanie ${r.key}`}>Usuń</Button>
                      </li>
                    ))}
                  </ul>
                )}
                <Button onClick={() => setSeverity({ ...sev, map: [...sev.map, { id: nextId(), key: "", value: "warning" }] })}>
                  Dodaj przypisanie wartości
                </Button>
              </>
            )}
            {sev.type === "rules" && (
              <>
                {sev.rules.length === 0 && <Text as="p" color="color.text.subtle">Brak warunków. Wybierz „Severity” nad drzewem i kliknij pole, albo dodaj regułę ręcznie.</Text>}
                <ul className="mp-rows">
                  {sev.rules.map((r, i) => (
                    <li key={r.id} className="mp-row mp-row-rule">
                      {isSimpleLeaf(r.when) ? (
                        <LeafEditor
                          cond={r.when}
                          label={`Reguła ${i + 1}`}
                          onChange={(when) => setSeverity({ ...sev, rules: sev.rules.map((x) => (x.id === r.id ? { ...x, when } : x)) })}
                        />
                      ) : (
                        <div className="mp-grow">
                          <ConditionView condition={r.when} />
                          <Text as="p" size="small" color="color.text.subtle">Złożony warunek – edytuj w pliku konfiguracji; tu można go tylko usunąć.</Text>
                        </div>
                      )}
                      <Field label="Wtedy severity">
                        {(f) => (
                          <SeveritySelect
                            {...f}
                            value={r.value}
                            aria-label={`Reguła ${i + 1}: severity`}
                            onChange={(value) => setSeverity({ ...sev, rules: sev.rules.map((x) => (x.id === r.id ? { ...x, value } : x)) })}
                          />
                        )}
                      </Field>
                      <Button onClick={() => setSeverity({ ...sev, rules: sev.rules.filter((x) => x.id !== r.id) })} aria-label={`Usuń regułę ${i + 1}`}>Usuń</Button>
                    </li>
                  ))}
                </ul>
                <Button
                  onClick={() =>
                    setSeverity({ ...sev, rules: [...sev.rules, { id: nextId(), when: { field: "raw.", op: "eq", value: "" }, value: "critical" }] })
                  }
                >
                  Dodaj regułę
                </Button>
                <Text as="p" size="small" color="color.text.subtle">Pierwsza spełniona reguła wygrywa.</Text>
              </>
            )}
            {sev.type !== "const" && (
              <Field label="Gdy nic nie pasuje">
                {(f) => (
                  <SeveritySelect {...f} value={sev.default} onChange={(value) => setSeverity({ ...sev, default: value })} aria-label="Severity domyślne" />
                )}
              </Field>
            )}
            {submitted && showErr(problems.severity)}
          </Section>

          <Section title="Fingerprint (deduplikacja)" mode="fingerprint" active={mode} onPick={setMode}>
            <Text as="p" size="small" color="color.text.subtle">
              Zdarzenia o tych samych wartościach wskazanych pól łączą się w jeden alert. Wybierz pola opisujące
              „ten sam problem” (np. usługa + kod błędu), nie identyfikator pojedynczego uruchomienia.
            </Text>
            {draft.fingerprint.length === 0 ? (
              <SectionMessage appearance="warning">
                Pusty fingerprint: liczony jest ze schematu i całego payloadu – dedup połączy tylko identyczne zdarzenia.
              </SectionMessage>
            ) : (
              <ul className="mp-rows">
                {draft.fingerprint.map((p, i) => (
                  <li key={i}>
                    <div className="mp-row">
                      <div className="mp-grow">
                        <Field label={`Ścieżka ${i + 1}`}>
                          {(f) => (
                            <TextInput
                              {...f}
                              className="mono"
                              value={p}
                              onChange={(e) => set({ fingerprint: draft.fingerprint.map((x, j) => (j === i ? e.currentTarget.value : x)) })}
                            />
                          )}
                        </Field>
                      </div>
                      <Button onClick={() => set({ fingerprint: draft.fingerprint.filter((_, j) => j !== i) })} aria-label={`Usuń ścieżkę fingerprintu ${i + 1}`}>Usuń</Button>
                    </div>
                    {showErr(problems.fingerprint[i])}
                  </li>
                ))}
              </ul>
            )}
          </Section>

          <Section title="Warunek resolved" mode="resolved" active={mode} onPick={setMode}>
            <Checkbox
              isChecked={draft.resolved !== null}
              onChange={(e) => set({ resolved: e.currentTarget.checked ? { field: "raw.", op: "eq", value: "" } : null })}
              label="Zdarzenie może rozwiązywać alert (np. status = resolved)"
            />
            {draft.resolved && isSimpleLeaf(draft.resolved) && (
              <LeafEditor cond={draft.resolved} label="Resolved" onChange={(resolved) => set({ resolved })} />
            )}
            {draft.resolved && !isSimpleLeaf(draft.resolved) && (
              <>
                <ConditionView condition={draft.resolved} />
                <Button onClick={() => set({ resolved: { field: "raw.", op: "eq", value: "" } })}>Zastąp prostym warunkiem</Button>
              </>
            )}
            {showErr(problems.resolved)}
          </Section>

          <Section step="Krok 4" title="Podgląd na przykładowym zdarzeniu">
            <PreviewPanel mapping={mapping} payload={payload} blocked={blocked} />
          </Section>

          <Section step="Krok 5" title="Zapisz">
            {submitted && problems.any && <ErrorBox message="Popraw zaznaczone błędy w formularzu, żeby zapisać." />}
            {saveError && <ErrorBox message={`Serwer odrzucił mapping: ${saveError}`} />}
            <div className="mp-actions">
              <Button appearance="primary" onClick={save} isDisabled={saving}>
                {saving ? "Zapisywanie…" : isNew ? "Zapisz mapping" : "Zapisz zmiany"}
              </Button>
              <Button onClick={onClose} isDisabled={saving}>
                Anuluj
              </Button>
            </div>
          </Section>
        </div>
      </div>
    </div>
  );
}
