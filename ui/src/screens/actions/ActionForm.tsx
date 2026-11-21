// Formularz dodawania/edycji akcji deklaratywnej HTTP (HttpActionSpec + ActionDef z kontraktu).
// Bezpieczeństwo (CLAUDE.md, zasada 6): risk=high zawsze wymaga zatwierdzenia, host musi być na allowliście,
// sekrety wyłącznie jako ${ENV}. Walidacja po stronie backendu jest rozstrzygająca – jej błędy pokazujemy przy polach.
import { Text } from "@atlaskit/primitives/compiled";
import Button from "@atlaskit/button/new";
import { Code } from "@atlaskit/code";
import SectionMessage from "@atlaskit/section-message";
import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { api, type Schemas } from "../../api/client";
import { Card, ErrorBox, Field, Select, TextArea, TextInput } from "../../ui/primitives";
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

export default function ActionForm({ initial, isNew, version, takenNames, onSaved, onCancel }: Props) {
  const headersHintId = useId();
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


  const title = isNew ? "Nowa akcja HTTP" : `Edycja akcji ${initial.name}`;

  return (
    <Card title={title} className="af">
      <form className="af-form" ref={rootRef} onSubmit={(e) => void submit(e)} noValidate aria-label={title}>
        {topError && <ErrorBox message={topError} />}

        <div className="af-grid">
          <Field label="Nazwa" error={errors.name} hint="Tej nazwy używają kroki runbooków (pole action).">
            {(p) => (
              <TextInput
                {...p}
                ref={firstRef}
                name="name"
                value={f.name}
                isReadOnly={!isNew}
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => set("name", e.currentTarget.value)}
              />
            )}
          </Field>

          <Field label="Ryzyko" hint="Akcja o ryzyku „wysokie” zawsze wymaga zatwierdzenia przez człowieka — niezależnie od ustawień runbooka.">
            {(p) => (
              <Select
                {...p}
                value={f.risk}
                options={[
                  { value: "low", label: "niskie" },
                  { value: "high", label: "wysokie — zawsze z zatwierdzeniem" },
                ]}
                onChange={(v) => set("risk", v as FormState["risk"])}
              />
            )}
          </Field>
        </div>

        <Field label="Opis">
          {(p) => <TextInput {...p} name="description" value={f.description} onChange={(e) => set("description", e.currentTarget.value)} />}
        </Field>

        <div className="af-grid af-grid-url">
          <Field label="Metoda">
            {(p) => (
              <Select {...p} value={f.method} options={METHODS.map((m) => ({ value: m, label: m }))} onChange={(v) => set("method", v)} />
            )}
          </Field>
          <Field
            label="URL (szablon)"
            error={urlError}
            hint={
              <>
                Host musi być na allowliście (<Code>ACTION_HOST_ALLOWLIST</Code>), inaczej wykonanie kończy się błędem „host
                niedozwolony”. Wartości z alertu i parametrów kroku: <Code>{"{{service}}"}</Code>.
              </>
            }
          >
            {(p) => (
              <TextInput
                {...p}
                name="url"
                value={f.url}
                placeholder="http://mock-targets:8090/cache/{{service}}/flush"
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => set("url", e.currentTarget.value)}
              />
            )}
          </Field>
        </div>

        <fieldset className="af-headers" aria-describedby={headersHintId}>
          <legend>Nagłówki</legend>
          <div className="field-hint" id={headersHintId}>
            Sekrety podawaj wyłącznie jako referencję <Code>{"${ENV_NAME}"}</Code>, np.{" "}
            <Code>{"Bearer ${CACHE_ADMIN_TOKEN}"}</Code> — wartość jest pobierana ze środowiska w runtime i nigdy nie jest
            zapisywana w definicji akcji.
          </div>
          {f.headers.length === 0 && <Text as="p" color="color.text.subtle">Brak nagłówków.</Text>}
          {f.headers.map((h, i) => {
            const secret = looksLikePastedSecret(h.name, h.value);
            return (
              <div className="af-header-row" key={h.id}>
                <div className="af-header-inputs">
                  <TextInput
                    aria-label={`Nazwa nagłówka ${i + 1}`}
                    value={h.name}
                    placeholder="Authorization"
                    autoComplete="off"
                    spellCheck={false}
                        onChange={(e) => setHeader(h.id, { name: e.currentTarget.value })}
                  />
                  <TextInput
                    aria-label={`Wartość nagłówka ${i + 1}`}
                    value={h.value}
                    placeholder="Bearer ${CACHE_ADMIN_TOKEN}"
                    autoComplete="off"
                    spellCheck={false}
                        aria-invalid={secret}
                    onChange={(e) => setHeader(h.id, { value: e.currentTarget.value })}
                  />
                  <Button
                    aria-label={`Usuń nagłówek ${h.name || i + 1}`}
                    onClick={() => setF((cur) => ({ ...cur, headers: cur.headers.filter((x) => x.id !== h.id) }))}
                  >
                    Usuń
                  </Button>
                </div>
                {secret && (
                  <div role="status">
                    <SectionMessage appearance="warning">
                      Ta wartość wygląda na wklejony sekret. Użyj referencji <Code>{"${ENV_NAME}"}</Code> zamiast wartości —
                      sekret nie powinien trafić do definicji akcji.
                    </SectionMessage>
                  </div>
                )}
              </div>
            );
          })}
          {errors.headers && (
            <div className="field-error" role="alert">
              {errors.headers}
            </div>
          )}
          <Button onClick={() => set("headers", [...f.headers, newRow()])}>Dodaj nagłówek</Button>
        </fieldset>

        <Field
          label="Treść żądania (szablon)"
          hint={
            <>
              Opcjonalna. Szablony jak w URL, np. <Code>{'{"service": "{{service}}"}'}</Code>. Bez dostępu do plików i
              zmiennych środowiskowych.
            </>
          }
        >
          {(p) => <TextArea {...p} name="body" rows={4} value={f.body} spellCheck={false} onChange={(e) => set("body", e.currentTarget.value)} />}
        </Field>

        <div className="af-grid">
          <Field label="Timeout (ms)" error={errors.timeout} hint="Po tym czasie krok kończy się błędem timeoutu.">
            {(p) => <TextInput {...p} name="timeout" inputMode="numeric" value={f.timeout} onChange={(e) => set("timeout", e.currentTarget.value)} />}
          </Field>
        </div>

        <Field
          label="Schemat parametrów (JSON Schema)"
          error={errors.paramsSchema}
          hint={
            <>
              Opcjonalny. Opisuje parametry kroku, np. <Code>{'{"type":"object","required":["service"]}'}</Code>.
            </>
          }
        >
          {(p) => (
            <TextArea
              {...p}
              name="params_schema"
              rows={4}
              value={f.paramsSchema}
              spellCheck={false}
             
              onChange={(e) => set("paramsSchema", e.currentTarget.value)}
            />
          )}
        </Field>

        <div className="af-actions">
          <Button type="submit" appearance="primary" isLoading={saving}>
            {saving ? "Zapisywanie…" : isNew ? "Dodaj akcję" : "Zapisz zmiany"}
          </Button>
          <Button isDisabled={saving} onClick={onCancel}>
            Anuluj
          </Button>
        </div>
      </form>
    </Card>
  );
}
