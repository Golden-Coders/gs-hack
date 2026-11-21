// Edytor mappingu krok po kroku: schemat -> zaznacz pola w payloadzie -> severity/fingerprint/resolved -> podgląd -> zapis.
// Kliknięcie w drzewie payloadu dodaje pole do aktualnie wybranego celu (pola, tytuł, fingerprint, severity, resolved).
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import ConditionView from "../../components/ConditionView";
import PayloadTree from "../../components/PayloadTree";
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

function describeError(e: ApiError, status: number): string {
  if (!e) return `Serwer odpowiedział błędem ${status}.`;
  const parts = [e.error, e.detail].filter(Boolean).join(": ");
  return e.line ? `${parts} (linia ${e.line})` : parts || `Serwer odpowiedział błędem ${status}.`;
}

function SeveritySelect({ value, onChange, label }: { value: Severity; onChange: (s: Severity) => void; label: string }) {
  return (
    <select aria-label={label} value={value} onChange={(e) => onChange(e.target.value as Severity)}>
      {SEVERITIES.map((s) => (
        <option key={s} value={s}>
          {s}
        </option>
      ))}
    </select>
  );
}

/** Wiersz warunku-liścia: pole, operator, wartość. */
function LeafEditor({ cond, onChange, label }: { cond: Cond & { field: string }; onChange: (c: Cond) => void; label: string }) {
  const op = (cond.op ?? "eq") as SimpleOp;
  return (
    <div className="mp-leaf">
      <label className="mp-lbl">
        <span>Pole payloadu</span>
        <input
          className="mono"
          value={cond.field}
          onChange={(e) => onChange({ ...cond, field: e.target.value })}
          aria-label={`${label}: pole`}
          placeholder="raw.status"
        />
      </label>
      <label className="mp-lbl">
        <span>Operator</span>
        <select
          value={op}
          onChange={(e) => onChange({ ...cond, op: e.target.value as SimpleOp })}
          aria-label={`${label}: operator`}
        >
          {SIMPLE_OPS.map((o) => (
            <option key={o} value={o}>
              {OP_LABEL[o]}
            </option>
          ))}
        </select>
      </label>
      {op !== "exists" && (
        <label className="mp-lbl">
          <span>Wartość</span>
          <input
            value={valueToText(cond.value)}
            onChange={(e) => onChange({ ...cond, value: coerceValue(e.target.value, cond.value, op) })}
            aria-label={`${label}: wartość`}
          />
        </label>
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
    <section className="mp-section">
      <header>
        <h3>
          {step && <span className="mp-step">{step}</span>}
          {title}
        </h3>
        {mode && onPick && (
          <button type="button" aria-pressed={active === mode} onClick={() => onPick(mode)} className="mp-pick-btn">
            {active === mode ? "Wybierasz z drzewa" : "Wybierz z drzewa"}
          </button>
        )}
      </header>
      {children}
    </section>
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
      else setSaveError(describeError(error as ApiError, response.status));
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
  const showErr = (msg?: string) => (msg ? <p className="mp-err" role="alert">{msg}</p> : null);
  const modeInfo = MODES.find((m) => m.id === mode)!;
  const sev = draft.severity;

  return (
    <div className="mp-editor">
      <header className="mp-editor-head">
        <div>
          <h2>
            {isNew ? "Nowy mapping" : "Edycja mappingu"}: <span className="mono">{draft.schema || "…"}</span>
          </h2>
          <p className="muted">
            {isNew
              ? "Schemat wybrany (krok 1 z 5). Zaznacz pola w payloadzie, ustaw severity i fingerprint, sprawdź podgląd i zapisz."
              : `Wersja ${draft.version ?? "?"}. Zmiana działa na kolejne zdarzenia bez restartu.`}
            {eventCount !== undefined && ` Czekających zdarzeń UNMAPPED: ${eventCount} – po zapisie zostaną przetworzone ponownie.`}
          </p>
        </div>
        <button type="button" onClick={onClose} disabled={saving}>
          Zamknij edytor
        </button>
      </header>

      <div className="mp-grid">
        {/* ----------------------------------------------------------- lewa kolumna: payload */}
        <div className="mp-left">
          <Section step="Krok 2" title="Zaznacz pola w payloadzie">
            <div role="group" aria-label="Cel kliknięcia w drzewie" className="mp-modes">
              {MODES.map((m) => (
                <button key={m.id} type="button" aria-pressed={mode === m.id} onClick={() => setMode(m.id)}>
                  {m.label}
                </button>
              ))}
            </div>
            <p className="mp-hint" aria-live="polite">
              <strong>Klik dodaje do: {MODE_LABEL[mode]}.</strong> {modeInfo.hint}
            </p>
            {payload !== undefined ? (
              <div className="mp-tree-box">
                <PayloadTree value={payload} onSelect={pick} marks={marks} actionLabel={`Dodaj do: ${MODE_LABEL[mode]}`} />
              </div>
            ) : (
              <p className="empty">Brak przykładowego payloadu – wklej go poniżej (JSON lub YAML).</p>
            )}
            <details className="mp-sample" open={payload === undefined}>
              <summary>
                Przykładowy payload{parsed && parsed.format !== "invalid" ? ` (${parsed.format.toUpperCase()})` : ""} – zmień
              </summary>
              <label className="mp-lbl">
                <span>Wklej zdarzenie (JSON lub YAML)</span>
                <textarea
                  className="mono"
                  rows={8}
                  value={sampleText}
                  onChange={(e) => setSampleText(e.target.value)}
                  spellCheck={false}
                />
              </label>
              {parsed?.format === "invalid" && <p className="mp-err" role="alert">Nie udało się odczytać payloadu: {parsed.error}</p>}
            </details>
          </Section>
        </div>

        {/* ----------------------------------------------------------- prawa kolumna: ustawienia */}
        <div className="mp-right">
          <Section step="Krok 3" title="Pola kanoniczne i tytuł" mode="fields" active={mode} onPick={setMode}>
            {draft.fields.length === 0 ? (
              <p className="muted">Brak pól. Wybierz „Pola” nad drzewem i kliknij pole payloadu.</p>
            ) : (
              <ul className="mp-rows">
                {draft.fields.map((f) => (
                  <li key={f.id}>
                    <div className="mp-row">
                      <label className="mp-lbl">
                        <span>Nazwa pola</span>
                        <input
                          value={f.name}
                          onChange={(e) => set({ fields: draft.fields.map((x) => (x.id === f.id ? { ...x, name: e.target.value } : x)) })}
                        />
                      </label>
                      <label className="mp-lbl mp-grow">
                        <span>Ścieżka</span>
                        <input
                          className="mono"
                          value={f.path}
                          onChange={(e) => set({ fields: draft.fields.map((x) => (x.id === f.id ? { ...x, path: e.target.value } : x)) })}
                        />
                      </label>
                      <button type="button" onClick={() => set({ fields: draft.fields.filter((x) => x.id !== f.id) })} aria-label={`Usuń pole ${f.name}`}>
                        Usuń
                      </button>
                    </div>
                    {showErr(problems.fields[f.id])}
                  </li>
                ))}
              </ul>
            )}
            <div className="mp-lbl">
              <label htmlFor="mp-title">Tytuł alertu (ścieżka lub szablon, np. {"{{raw.error.message}}"})</label>
              <div className="mp-inline">
                <input
                  id="mp-title"
                  className="mono mp-grow"
                  value={draft.title}
                  onChange={(e) => set({ title: e.target.value })}
                  placeholder="$.error.message"
                />
                <button type="button" aria-pressed={mode === "title"} onClick={() => setMode("title")} className="mp-pick-btn">
                  Wybierz z drzewa
                </button>
              </div>
            </div>
            {showErr(problems.title)}
            {!draft.title.trim() && <p className="muted mp-small">Bez tytułu alert dostanie nazwę schematu.</p>}
          </Section>

          <Section title="Severity" mode="severity" active={mode} onPick={setMode}>
            <div className="mp-radios" role="radiogroup" aria-label="Skąd brać severity">
              {(
                [
                  ["const", "Stała wartość"],
                  ["path", "Z pola payloadu"],
                  ["rules", "Z warunków"],
                ] as const
              ).map(([t, label]) => (
                <label key={t}>
                  <input
                    type="radio"
                    name="sev-type"
                    checked={sev.type === t}
                    onChange={() => {
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
                  {label}
                </label>
              ))}
            </div>
            {sev.type === "const" && (
              <label className="mp-lbl">
                <span>Severity wszystkich zdarzeń</span>
                <SeveritySelect value={sev.value} onChange={(value) => setSeverity({ type: "const", value })} label="Severity" />
              </label>
            )}
            {sev.type === "path" && (
              <>
                <label className="mp-lbl">
                  <span>Pole z severity</span>
                  <input
                    className="mono"
                    value={sev.path}
                    onChange={(e) => setSeverity({ ...sev, path: e.target.value })}
                    placeholder="$.severity"
                  />
                </label>
                <p className="muted mp-small">
                  Nazwy critical, error, warning/warn, info są rozpoznawane same. Inne wartości źródła przypisz poniżej.
                </p>
                {sev.map.length > 0 && (
                  <ul className="mp-rows">
                    {sev.map.map((r) => (
                      <li key={r.id} className="mp-row">
                        <label className="mp-lbl mp-grow">
                          <span>Wartość w źródle</span>
                          <input
                            value={r.key}
                            onChange={(e) => setSeverity({ ...sev, map: sev.map.map((x) => (x.id === r.id ? { ...x, key: e.target.value } : x)) })}
                          />
                        </label>
                        <label className="mp-lbl">
                          <span>Severity</span>
                          <SeveritySelect
                            value={r.value}
                            label={`Severity dla ${r.key || "wartości"}`}
                            onChange={(value) => setSeverity({ ...sev, map: sev.map.map((x) => (x.id === r.id ? { ...x, value } : x)) })}
                          />
                        </label>
                        <button type="button" onClick={() => setSeverity({ ...sev, map: sev.map.filter((x) => x.id !== r.id) })} aria-label={`Usuń przypisanie ${r.key}`}>
                          Usuń
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <button type="button" onClick={() => setSeverity({ ...sev, map: [...sev.map, { id: nextId(), key: "", value: "warning" }] })}>
                  Dodaj przypisanie wartości
                </button>
              </>
            )}
            {sev.type === "rules" && (
              <>
                {sev.rules.length === 0 && <p className="muted">Brak warunków. Wybierz „Severity” nad drzewem i kliknij pole, albo dodaj regułę ręcznie.</p>}
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
                          <p className="muted mp-small">Złożony warunek – edytuj w pliku konfiguracji; tu można go tylko usunąć.</p>
                        </div>
                      )}
                      <label className="mp-lbl">
                        <span>Wtedy severity</span>
                        <SeveritySelect
                          value={r.value}
                          label={`Reguła ${i + 1}: severity`}
                          onChange={(value) => setSeverity({ ...sev, rules: sev.rules.map((x) => (x.id === r.id ? { ...x, value } : x)) })}
                        />
                      </label>
                      <button type="button" onClick={() => setSeverity({ ...sev, rules: sev.rules.filter((x) => x.id !== r.id) })} aria-label={`Usuń regułę ${i + 1}`}>
                        Usuń
                      </button>
                    </li>
                  ))}
                </ul>
                <button
                  type="button"
                  onClick={() =>
                    setSeverity({ ...sev, rules: [...sev.rules, { id: nextId(), when: { field: "raw.", op: "eq", value: "" }, value: "critical" }] })
                  }
                >
                  Dodaj regułę
                </button>
                <p className="muted mp-small">Pierwsza spełniona reguła wygrywa.</p>
              </>
            )}
            {sev.type !== "const" && (
              <label className="mp-lbl">
                <span>Gdy nic nie pasuje</span>
                <SeveritySelect value={sev.default} onChange={(value) => setSeverity({ ...sev, default: value })} label="Severity domyślne" />
              </label>
            )}
            {submitted && showErr(problems.severity)}
          </Section>

          <Section title="Fingerprint (deduplikacja)" mode="fingerprint" active={mode} onPick={setMode}>
            <p className="muted mp-small">
              Zdarzenia o tych samych wartościach wskazanych pól łączą się w jeden alert. Wybierz pola opisujące
              „ten sam problem” (np. usługa + kod błędu), nie identyfikator pojedynczego uruchomienia.
            </p>
            {draft.fingerprint.length === 0 ? (
              <p className="mp-note">
                Pusty fingerprint: liczony jest ze schematu i całego payloadu – dedup połączy tylko identyczne zdarzenia.
              </p>
            ) : (
              <ul className="mp-rows">
                {draft.fingerprint.map((p, i) => (
                  <li key={i}>
                    <div className="mp-row">
                      <label className="mp-lbl mp-grow">
                        <span>Ścieżka {i + 1}</span>
                        <input
                          className="mono"
                          value={p}
                          onChange={(e) => set({ fingerprint: draft.fingerprint.map((x, j) => (j === i ? e.target.value : x)) })}
                        />
                      </label>
                      <button type="button" onClick={() => set({ fingerprint: draft.fingerprint.filter((_, j) => j !== i) })} aria-label={`Usuń ścieżkę fingerprintu ${i + 1}`}>
                        Usuń
                      </button>
                    </div>
                    {showErr(problems.fingerprint[i])}
                  </li>
                ))}
              </ul>
            )}
          </Section>

          <Section title="Warunek resolved" mode="resolved" active={mode} onPick={setMode}>
            <label className="mp-check">
              <input
                type="checkbox"
                checked={draft.resolved !== null}
                onChange={(e) => set({ resolved: e.target.checked ? { field: "raw.", op: "eq", value: "" } : null })}
              />
              Zdarzenie może rozwiązywać alert (np. status = resolved)
            </label>
            {draft.resolved && isSimpleLeaf(draft.resolved) && (
              <LeafEditor cond={draft.resolved} label="Resolved" onChange={(resolved) => set({ resolved })} />
            )}
            {draft.resolved && !isSimpleLeaf(draft.resolved) && (
              <>
                <ConditionView condition={draft.resolved} />
                <button type="button" onClick={() => set({ resolved: { field: "raw.", op: "eq", value: "" } })}>
                  Zastąp prostym warunkiem
                </button>
              </>
            )}
            {showErr(problems.resolved)}
          </Section>

          <Section step="Krok 4" title="Podgląd na przykładowym zdarzeniu">
            <PreviewPanel mapping={mapping} payload={payload} blocked={blocked} />
          </Section>

          <Section step="Krok 5" title="Zapisz">
            {submitted && problems.any && <p className="error-box" role="alert">Popraw zaznaczone błędy w formularzu, żeby zapisać.</p>}
            {saveError && <p className="error-box" role="alert">Serwer odrzucił mapping: {saveError}</p>}
            <div className="mp-actions">
              <button type="button" className="primary" onClick={save} disabled={saving}>
                {saving ? "Zapisywanie…" : isNew ? "Zapisz mapping" : "Zapisz zmiany"}
              </button>
              <button type="button" onClick={onClose} disabled={saving}>
                Anuluj
              </button>
            </div>
          </Section>
        </div>
      </div>
    </div>
  );
}
