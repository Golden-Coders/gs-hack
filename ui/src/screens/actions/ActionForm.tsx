// Formularz dodawania/edycji akcji deklaratywnej HTTP (HttpActionSpec + ActionDef z kontraktu).
// Bezpieczeństwo (CLAUDE.md, zasada 6): risk=high zawsze wymaga zatwierdzenia, host musi być na allowliście,
// sekrety wyłącznie jako ${ENV}. Walidacja po stronie backendu jest rozstrzygająca – jej błędy pokazujemy przy polach.
import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { api, type Schemas } from "../../api/client";
import {
  METHODS,
  newRow,
  toAction,
  validate,
  looksLikePastedSecret,
  type ActionDef,
  type FieldErrors,
  type FormState,
} from "./model";

type Props = {
  initial: FormState;
  /** true = nowa akcja (POST), false = edycja istniejącej (PUT). */
  isNew: boolean;
  version?: number;
  /** Nazwy już zajęte w rejestrze (walidacja duplikatów przy dodawaniu). */
  takenNames: Set<string>;
  onSaved: (a: ActionDef) => void;
  onCancel: () => void;
};

type ApiError = Schemas["Error"];

function Field({
  id,
  label,
  hint,
  error,
  children,
}: {
  id: string;
  label: string;
  hint?: React.ReactNode;
  error?: string | null;
  children: React.ReactNode;
}) {
  return (
    <div className="af-field">
      <label htmlFor={id}>{label}</label>
      {children}
      {hint && (
        <p className="af-hint" id={`${id}-hint`}>
          {hint}
        </p>
      )}
      {error && (
        <p className="af-error" id={`${id}-err`} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export default function ActionForm({ initial, isNew, version, takenNames, onSaved, onCancel }: Props) {
  const uid = useId();
  const id = (s: string) => `${uid}-${s}`;
  const [f, setF] = useState<FormState>(initial);
  const [errors, setErrors] = useState<FieldErrors>({});
  const [serverError, setServerError] = useState<ApiError | null>(null);
  const [serverField, setServerField] = useState<"url" | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const firstRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLFormElement>(null);

  // Po otwarciu formularza (np. „Dodaj” przy brakującej akcji) przewijamy do niego i ustawiamy fokus.
  useEffect(() => {
    rootRef.current?.scrollIntoView({ block: "nearest" });
    (isNew && !initial.name ? firstRef.current : rootRef.current?.querySelector<HTMLInputElement>("[name=url]"))?.focus();
  }, [initial, isNew]);

  const set = <K extends keyof FormState>(k: K, v: FormState[K]) => {
    setF((cur) => ({ ...cur, [k]: v }));
    if (k in errors) setErrors((e) => ({ ...e, [k]: undefined }));
    if (k === "url") setServerField(null);
  };
  const setHeader = (rowId: number, patch: Partial<{ name: string; value: string }>) => {
    setF((cur) => ({ ...cur, headers: cur.headers.map((h) => (h.id === rowId ? { ...h, ...patch } : h)) }));
    setErrors((e) => ({ ...e, headers: undefined }));
  };

  async function submit(ev: FormEvent) {
    ev.preventDefault();
    setFormError(null);
    setServerError(null);
    setServerField(null);
    const errs = validate(f, { isNew, takenNames });
    setErrors(errs);
    if (Object.keys(errs).length > 0) {
      // Fokus na pierwsze błędne pole – po renderze, gdy aria-invalid jest już ustawione.
      setTimeout(() => rootRef.current?.querySelector<HTMLElement>("[aria-invalid=true]")?.focus(), 0);
      return;
    }
    const body = toAction(f, version);
    setSaving(true);
    try {
      const res = isNew
        ? await api.POST("/actions", { body })
        : await api.PUT("/actions/{name}", { params: { path: { name: body.name } }, body });
      if (res.response.ok) {
        // Odpowiedź serwera ma pierwszeństwo tylko, jeśli dotyczy tej samej akcji (np. zaktualizowana wersja).
        const saved = res.data as ActionDef | undefined;
        onSaved(saved && saved.name === body.name ? saved : body);
        return;
      }
      const err = res.error as ApiError | undefined;
      if (err?.error) {
        setServerError(err);
        // Błąd dotyczący adresu/hosta (np. spoza allowlisty) pokazujemy przy polu URL.
        if (/host|url|allowlist|adres/i.test(`${err.error} ${err.detail ?? ""}`)) setServerField("url");
      } else {
        setFormError(`Nie udało się zapisać akcji (HTTP ${res.response.status}).`);
      }
    } catch {
      setFormError("Brak połączenia z API — akcja nie została zapisana.");
    } finally {
      setSaving(false);
    }
  }

  const serverText = serverError ? [serverError.error, serverError.detail].filter(Boolean).join(": ") : null;
  const urlError = errors.url ?? (serverField === "url" ? serverText : null);
  const topError = formError ?? (serverError && serverField !== "url" ? serverText : null);

  return (
    <form className="af" ref={rootRef} onSubmit={(e) => void submit(e)} noValidate aria-labelledby={id("title")}>
      <h2 id={id("title")}>{isNew ? "Nowa akcja HTTP" : `Edycja akcji ${initial.name}`}</h2>

      {topError && (
        <p className="error-box" role="alert">
          {topError}
        </p>
      )}

      <div className="af-grid">
        <Field id={id("name")} label="Nazwa" error={errors.name} hint="Tej nazwy używają kroki runbooków (pole action).">
          <input
            ref={firstRef}
            id={id("name")}
            name="name"
            value={f.name}
            readOnly={!isNew}
            autoComplete="off"
            spellCheck={false}
            className="mono"
            aria-invalid={!!errors.name}
            aria-describedby={`${id("name")}-hint${errors.name ? ` ${id("name")}-err` : ""}`}
            onChange={(e) => set("name", e.target.value)}
          />
        </Field>

        <Field id={id("risk")} label="Ryzyko" hint="Akcja o ryzyku „wysokie” zawsze wymaga zatwierdzenia przez człowieka — niezależnie od ustawień runbooka.">
          <select
            id={id("risk")}
            name="risk"
            value={f.risk}
            aria-describedby={`${id("risk")}-hint`}
            onChange={(e) => set("risk", e.target.value as FormState["risk"])}
          >
            <option value="low">niskie</option>
            <option value="high">wysokie — zawsze z zatwierdzeniem</option>
          </select>
        </Field>
      </div>

      <Field id={id("desc")} label="Opis">
        <input id={id("desc")} name="description" value={f.description} onChange={(e) => set("description", e.target.value)} />
      </Field>

      <div className="af-grid af-grid-url">
        <Field id={id("method")} label="Metoda">
          <select id={id("method")} name="method" value={f.method} onChange={(e) => set("method", e.target.value)}>
            {METHODS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        </Field>
        <Field
          id={id("url")}
          label="URL (szablon)"
          error={urlError}
          hint={
            <>
              Host musi być na allowliście (<code>ACTION_HOST_ALLOWLIST</code>), inaczej wykonanie kończy się błędem „host
              niedozwolony”. Wartości z alertu i parametrów kroku: <code>{"{{service}}"}</code>.
            </>
          }
        >
          <input
            id={id("url")}
            name="url"
            value={f.url}
            placeholder="http://mock-targets:8090/cache/{{service}}/flush"
            autoComplete="off"
            spellCheck={false}
            className="mono"
            aria-invalid={!!urlError}
            aria-describedby={`${id("url")}-hint${urlError ? ` ${id("url")}-err` : ""}`}
            onChange={(e) => set("url", e.target.value)}
          />
        </Field>
      </div>

      <fieldset className="af-headers" aria-describedby={id("headers-hint")}>
        <legend>Nagłówki</legend>
        <p className="af-hint" id={id("headers-hint")}>
          Sekrety podawaj wyłącznie jako referencję <code>{"${ENV_NAME}"}</code>, np.{" "}
          <code>{"Bearer ${CACHE_ADMIN_TOKEN}"}</code> — wartość jest pobierana ze środowiska w runtime i nigdy nie jest
          zapisywana w definicji akcji.
        </p>
        {f.headers.length === 0 && <p className="muted">Brak nagłówków.</p>}
        {f.headers.map((h, i) => {
          const secret = looksLikePastedSecret(h.name, h.value);
          return (
            <div className="af-header-row" key={h.id}>
              <div className="af-header-inputs">
                <input
                  aria-label={`Nazwa nagłówka ${i + 1}`}
                  value={h.name}
                  placeholder="Authorization"
                  autoComplete="off"
                  spellCheck={false}
                  className="mono"
                  onChange={(e) => setHeader(h.id, { name: e.target.value })}
                />
                <input
                  aria-label={`Wartość nagłówka ${i + 1}`}
                  value={h.value}
                  placeholder="Bearer ${CACHE_ADMIN_TOKEN}"
                  autoComplete="off"
                  spellCheck={false}
                  className="mono"
                  aria-invalid={secret}
                  onChange={(e) => setHeader(h.id, { value: e.target.value })}
                />
                <button
                  type="button"
                  aria-label={`Usuń nagłówek ${h.name || i + 1}`}
                  onClick={() => setF((cur) => ({ ...cur, headers: cur.headers.filter((x) => x.id !== h.id) }))}
                >
                  Usuń
                </button>
              </div>
              {secret && (
                <p className="af-warn" role="status">
                  Ta wartość wygląda na wklejony sekret. Użyj referencji <code>{"${ENV_NAME}"}</code> zamiast wartości —
                  sekret nie powinien trafić do definicji akcji.
                </p>
              )}
            </div>
          );
        })}
        {errors.headers && (
          <p className="af-error" role="alert">
            {errors.headers}
          </p>
        )}
        <button type="button" onClick={() => set("headers", [...f.headers, newRow()])}>
          Dodaj nagłówek
        </button>
      </fieldset>

      <Field
        id={id("body")}
        label="Treść żądania (szablon)"
        hint={
          <>
            Opcjonalna. Szablony jak w URL, np. <code>{'{"service": "{{service}}"}'}</code>. Bez dostępu do plików i
            zmiennych środowiskowych.
          </>
        }
      >
        <textarea
          id={id("body")}
          name="body"
          rows={4}
          value={f.body}
          spellCheck={false}
          className="mono"
          aria-describedby={`${id("body")}-hint`}
          onChange={(e) => set("body", e.target.value)}
        />
      </Field>

      <div className="af-grid">
        <Field id={id("timeout")} label="Timeout (ms)" error={errors.timeout} hint="Po tym czasie krok kończy się błędem timeoutu.">
          <input
            id={id("timeout")}
            name="timeout"
            inputMode="numeric"
            value={f.timeout}
            className="mono"
            aria-invalid={!!errors.timeout}
            aria-describedby={`${id("timeout")}-hint${errors.timeout ? ` ${id("timeout")}-err` : ""}`}
            onChange={(e) => set("timeout", e.target.value)}
          />
        </Field>
      </div>

      <Field
        id={id("schema")}
        label="Schemat parametrów (JSON Schema)"
        error={errors.paramsSchema}
        hint={
          <>
            Opcjonalny. Opisuje parametry kroku, np. <code>{'{"type":"object","required":["service"]}'}</code>.
          </>
        }
      >
        <textarea
          id={id("schema")}
          name="params_schema"
          rows={4}
          value={f.paramsSchema}
          spellCheck={false}
          className="mono"
          aria-invalid={!!errors.paramsSchema}
          aria-describedby={`${id("schema")}-hint${errors.paramsSchema ? ` ${id("schema")}-err` : ""}`}
          onChange={(e) => set("paramsSchema", e.target.value)}
        />
      </Field>

      <div className="af-actions">
        <button type="submit" className="primary" disabled={saving}>
          {saving ? "Zapisywanie…" : isNew ? "Dodaj akcję" : "Zapisz zmiany"}
        </button>
        <button type="button" disabled={saving} onClick={onCancel}>
          Anuluj
        </button>
      </div>
    </form>
  );
}
