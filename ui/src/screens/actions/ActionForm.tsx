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
        setTopError(`Failed to save action (HTTP ${res.response.status}).`);
      }
    } catch {
      setTopError("Cannot connect to API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    if (!onDelete) return;
    setSaving(true);
    setTopError(null);
    const err = await onDelete(initial.name);
    if (err) setTopError(`Failed to delete action: ${err}`);
    setSaving(false);
  }

  return (
    <form className="act-editor-clean" onSubmit={(e) => void submit(e)} noValidate>
      <header className="act-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? (f.name ? `New action: ${f.name}` : "New action") : initial.name}
        </Heading>
      </header>

      {/* 1. Basic info */}
      <Card title="1. Basic information">
        <div className="act-two-col">
          <Field label="Action name (used in runbooks)" error={errors.name}>
            {(p) => (
              <TextInput
                {...p}
                value={f.name}
                placeholder="e.g. flush_cdn"
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => set("name", e.currentTarget.value)}
              />
            )}
          </Field>
          <Field label="Short description (what does it do?)">
            {(p) => (
              <TextInput
                {...p}
                value={f.description}
                placeholder="e.g. Flushes the service cache"
                onChange={(e) => set("description", e.currentTarget.value)}
              />
            )}
          </Field>
        </div>

        <div className="act-risk-box">
          <Segmented
            label="Execution mode"
            value={f.risk}
            onChange={(v) => set("risk", v)}
            options={[
              { value: "low", label: "Run automatically" },
              { value: "high", label: "Always require approval (high risk)" },
            ]}
          />
        </div>
      </Card>

      {/* 2. HTTP call */}
      <Card title="2. HTTP request">
        <div className="act-url-grid">
          <Field label="Method">
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
            label="URL"
            error={errors.url}
            hint="Use alert variables in double braces, e.g. http://mock-targets:8090/cache/{{service}}/flush"
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
              label="Request body (JSON — optional)"
              hint='e.g. {"service": "{{service}}"}'
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

      {/* 3. HTTP headers */}
      <Card title="3. Headers & authorization (optional)">
        {f.headers.length === 0 ? (
          <Text as="p" size="small" color="color.text.subtle">
            No custom headers. If the endpoint requires a token, add a header (e.g. Authorization: Bearer {"${CACHE_ADMIN_TOKEN}"}).
          </Text>
        ) : (
          <div className="act-headers-list">
            {f.headers.map((h) => {
              const secret = looksLikePastedSecret(h.name, h.value);
              return (
                <div key={h.id} className="act-header-item">
                  <div className="act-header-row">
                    <Field label="Header">
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
                    <Field label="Value">
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
                        Remove
                      </Button>
                    </div>
                  </div>
                  {secret && (
                    <SectionMessage appearance="warning">
                      Instead of pasting a raw token, reference an environment variable such as <code>{"${TOKEN_NAME}"}</code>.
                    </SectionMessage>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {errors.headers && <ErrorBox message={errors.headers} />}
        <div className="act-add-btn">
          <Button onClick={() => set("headers", [...f.headers, newRow()])}>+ Add header</Button>
        </div>
      </Card>

      {topError && <ErrorBox message={topError} />}

      <div className="act-savebar">
        <Button type="submit" appearance="primary" isLoading={saving} isDisabled={saving}>
          {isNew ? "Create action" : "Save changes"}
        </Button>
        {!isNew && onDelete && (
          <ConfirmInline
            label="Delete action"
            question={`Delete action "${initial.name}"?`}
            confirmLabel="Yes, delete"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </form>
  );
}
