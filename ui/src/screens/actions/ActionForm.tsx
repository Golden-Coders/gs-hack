// Maksymalnie prosty i przejrzysty formularz akcji HTTP:
// 1. Nazwa, opis i czy akcja wymaga zatwierdzenia przez człowieka
// 2. Wywołanie HTTP (Metoda, Adres URL, opcjonalna treść JSON)
// 3. Opcjonalne nagłówki (np. Authorization)
import { useState, type FormEvent } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import SectionMessage from "@atlaskit/section-message";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../../api/client";
import { Card, ConfirmInline, ErrorBox, Field, Segmented, Select, TextArea, TextInput } from "../../ui/primitives";
import {
  METHODS,
  looksLikePastedSecret,
  newRow,
  toAction,
  validate,
  type ActionDef,
  type FieldErrors,
  type FormState,
} from "./model";

type Props = {
  initial: FormState;
  isNew: boolean;
  version?: number;
  takenNames: Set<string>;
  onSaved: (a: ActionDef, oldName?: string) => void;
  onDelete?: (name: string) => Promise<string | null>;
};

type ApiError = Schemas["Error"];

export default function ActionForm({ initial, isNew, version, takenNames, onSaved, onDelete }: Props) {
  const [f, setF] = useState<FormState>(initial);
  const [errors, setErrors] = useState<FieldErrors>({});
  const [topError, setTopError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof FormState>(k: K, v: FormState[K]) => {
    setF((cur) => ({ ...cur, [k]: v }));
    if (k in errors) setErrors((e) => ({ ...e, [k]: undefined }));
  };

  const setHeader = (rowId: number, patch: Partial<{ name: string; value: string }>) => {
    setF((cur) => ({ ...cur, headers: cur.headers.map((h) => (h.id === rowId ? { ...h, ...patch } : h)) }));
    setErrors((e) => ({ ...e, headers: undefined }));
  };

  async function submit(ev: FormEvent) {
    ev.preventDefault();
    setTopError(null);
    const errs = validate(f, { isNew, originalName: initial.name, takenNames });
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;

    const body = toAction(f, version);
    setSaving(true);
    try {
      const res = isNew
        ? await api.POST("/actions", { body })
        : await api.PUT("/actions/{name}", { params: { path: { name: initial.name } }, body });
      if (res.response.ok) {
        const saved = res.data as ActionDef | undefined;
        onSaved(saved ?? body, isNew ? undefined : initial.name);
        return;
      }
      const err = res.error as ApiError | undefined;
      if (err?.error) {
        setTopError([err.error, err.detail].filter(Boolean).join(": "));
      } else {
        setTopError(`Nie udało się zapisać akcji (HTTP ${res.response.status}).`);
      }
    } catch {
      setTopError("Brak połączenia z API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    if (!onDelete) return;
    setSaving(true);
    setTopError(null);
    const err = await onDelete(initial.name);
    if (err) setTopError(`Nie udało się usunąć akcji: ${err}`);
    setSaving(false);
  }

  return (
    <form className="act-editor-clean" onSubmit={(e) => void submit(e)} noValidate>
      <header className="act-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? (f.name ? `Nowa akcja: ${f.name}` : "Nowa akcja") : initial.name}
        </Heading>
      </header>

      {/* 1. Nazwa, opis i tryb wykonywania */}
      <Card title="1. Podstawowe informacje">
        <div className="act-two-col">
          <Field label="Nazwa akcji (używana w runbookach)" error={errors.name}>
            {(p) => (
              <TextInput
                {...p}
                value={f.name}
                placeholder="np. flush_cdn"
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => set("name", e.currentTarget.value)}
              />
            )}
          </Field>
          <Field label="Krótki opis (co robi ta akcja?)">
            {(p) => (
              <TextInput
                {...p}
                value={f.description}
                placeholder="np. Czyści pamięć podręczną usługi"
                onChange={(e) => set("description", e.currentTarget.value)}
              />
            )}
          </Field>
        </div>

        <div className="act-risk-box">
          <Segmented
            label="Tryb wykonywania akcji"
            value={f.risk}
            onChange={(v) => set("risk", v)}
            options={[
              { value: "low", label: "Wykonuj automatycznie" },
              { value: "high", label: "Zawsze pytaj o zatwierdzenie (wysokie ryzyko)" },
            ]}
          />
        </div>
      </Card>

      {/* 2. Wywołanie HTTP */}
      <Card title="2. Wywołanie HTTP">
        <div className="act-url-grid">
          <Field label="Metoda">
            {(p) => (
              <Select
                {...p}
                value={f.method}
                options={METHODS.map((m) => ({ value: m, label: m }))}
                onChange={(v) => set("method", v)}
              />
            )}
          </Field>
          <Field
            label="Adres URL"
            error={errors.url}
            hint="Możesz używać zmiennych z alertu w podwójnych nawiasach, np. http://mock-targets:8090/cache/{{service}}/flush"
          >
            {(p) => (
              <TextInput
                {...p}
                value={f.url}
                placeholder="http://mock-targets:8090/..."
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => set("url", e.currentTarget.value)}
              />
            )}
          </Field>
        </div>

        {f.method !== "GET" && (
          <div className="act-body-box">
            <Field
              label="Dane wysyłane w żądaniu (JSON — opcjonalnie)"
              hint='Np. {"service": "{{service}}"}'
            >
              {(p) => (
                <TextArea
                  {...p}
                  rows={3}
                  value={f.body}
                  spellCheck={false}
                  placeholder='{"service": "{{service}}"}'
                  onChange={(e) => set("body", e.currentTarget.value)}
                />
              )}
            </Field>
          </div>
        )}
      </Card>

      {/* 3. Nagłówki HTTP (opcjonalne) */}
      <Card title="3. Nagłówki i autoryzacja (opcjonalnie)">
        {f.headers.length === 0 ? (
          <Text as="p" size="small" color="color.text.subtle">
            Brak dodatkowych nagłówków. Jeśli endpoint wymaga tokenu, dodaj nagłówek (np. Authorization: Bearer {"${CACHE_ADMIN_TOKEN}"}).
          </Text>
        ) : (
          <div className="act-headers-list">
            {f.headers.map((h, idx) => {
              const secret = looksLikePastedSecret(h.name, h.value);
              return (
                <div key={h.id} className="act-header-item">
                  <div className="act-header-row">
                    <Field label={idx === 0 ? "Nagłówek" : "Nagłówek"}>
                      {(p) => (
                        <TextInput
                          {...p}
                          value={h.name}
                          placeholder="Authorization"
                          autoComplete="off"
                          spellCheck={false}
                          onChange={(e) => setHeader(h.id, { name: e.currentTarget.value })}
                        />
                      )}
                    </Field>
                    <Field label={idx === 0 ? "Wartość" : "Wartość"}>
                      {(p) => (
                        <TextInput
                          {...p}
                          value={h.value}
                          placeholder="Bearer ${CACHE_ADMIN_TOKEN}"
                          autoComplete="off"
                          spellCheck={false}
                          onChange={(e) => setHeader(h.id, { value: e.currentTarget.value })}
                        />
                      )}
                    </Field>
                    <div className="act-row-btn">
                      <Button
                        appearance="subtle"
                        onClick={() => setF((cur) => ({ ...cur, headers: cur.headers.filter((x) => x.id !== h.id) }))}
                      >
                        Usuń
                      </Button>
                    </div>
                  </div>
                  {secret && (
                    <SectionMessage appearance="warning">
                      Zamiast wklejać jawny token, użyj zmiennej środowiskowej, np. <code>{"${NAZWA_TOKENU}"}</code>.
                    </SectionMessage>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {errors.headers && <ErrorBox message={errors.headers} />}
        <div className="act-add-btn">
          <Button onClick={() => set("headers", [...f.headers, newRow()])}>+ Dodaj nagłówek</Button>
        </div>
      </Card>

      {topError && <ErrorBox message={topError} />}

      <div className="act-savebar">
        <Button type="submit" appearance="primary" isLoading={saving} isDisabled={saving}>
          {isNew ? "Utwórz akcję" : "Zapisz zmiany"}
        </Button>
        {!isNew && onDelete && (
          <ConfirmInline
            label="Usuń akcję"
            question={`Usunąć akcję „${initial.name}”?`}
            confirmLabel="Tak, usuń"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </form>
  );
}
