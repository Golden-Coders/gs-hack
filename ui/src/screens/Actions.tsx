import { useEffect, useRef, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import { api, type Schemas } from "../api/client";
import { send } from "../api/request";
import { useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Card, Empty, ErrorBox, Loading, PageHeader } from "../ui/primitives";
import ActionForm from "./actions/ActionForm";
import { emptyForm, fromAction, type ActionDef } from "./actions/model";
import "./Actions.css";

type Missing = Schemas["MissingAction"];

const ADDED_HIDE_MS = 15_000;

export default function Actions() {
  const added = useRef(new Map<string, { at: number; action: ActionDef | null }>());
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [creatingName, setCreatingName] = useState<string | null>(null);

  const resource = useResource(async () => {
    const [a, m] = await Promise.all([api.GET("/actions"), api.GET("/actions/missing")]);
    if (a.data === undefined) return { error: a.error, response: a.response };
    if (m.data === undefined) return { error: m.error, response: m.response };
    const now = Date.now();
    const fresh = [...added.current.values()].filter((x) => x.action && now - x.at <= ADDED_HIDE_MS);
    const have = new Set(a.data.map((x) => x.name));
    const merged = [...a.data, ...fresh.map((x) => x.action as ActionDef).filter((x) => !have.has(x.name))];
    return {
      data: {
        actions: merged.sort((x, y) => x.name.localeCompare(y.name)),
        missing: m.data.filter((x: Missing) => now - (added.current.get(x.name)?.at ?? -Infinity) > ADDED_HIDE_MS),
      },
      response: a.response,
    };
  });

  const actions = resource.data?.actions ?? [];
  const missing = resource.data?.missing ?? [];

  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "config_changed") resource.refresh();
  });

  useEffect(() => {
    if (selectedName !== null || creatingName !== null) return;
    if (actions.length > 0) {
      const firstHttp = actions.find((a) => a.kind === "http") ?? actions[0];
      setSelectedName(firstHttp.name);
    }
  }, [actions, selectedName, creatingName]);

  const takenNames = new Set(actions.map((a) => a.name));
  const visibleMissing = missing.filter((m) => !takenNames.has(m.name));

  const selectedAction =
    creatingName === null && selectedName ? (actions.find((a) => a.name === selectedName) ?? null) : null;

  function handleSelect(a: ActionDef) {
    setCreatingName(null);
    setSelectedName(a.name);
  }

  function handleStartNew(prefillName = "") {
    setSelectedName(null);
    setCreatingName(prefillName);
  }

  function handleSaved(saved: ActionDef, oldName?: string) {
    if (oldName && oldName !== saved.name) {
      added.current.delete(oldName);
    }
    added.current.set(saved.name, { at: Date.now(), action: saved });
    setCreatingName(null);
    setSelectedName(saved.name);
    resource.refresh();
  }

  async function handleDelete(name: string): Promise<string | null> {
    const err = await send(() => api.DELETE("/actions/{name}", { params: { path: { name } } }));
    if (!err) {
      added.current.delete(name);
      const remaining = actions.filter((a) => a.name !== name);
      setSelectedName(remaining[0]?.name ?? null);
      setCreatingName(null);
      resource.refresh();
    }
    return err;
  }

  return (
    <section className="act-screen">
      <PageHeader
        actions={
          <Button appearance="primary" onClick={() => handleStartNew("")}>
            + Nowa akcja
          </Button>
        }
      >
        Akcje
      </PageHeader>

      {resource.error && <ErrorBox message={resource.error} onRetry={resource.reload} />}

      {/* Kompaktowy baner brakujących akcji w runbookach (tylko jeśli są jakieś nieobsłużone, z pominięciem demo `shell`) */}
      {visibleMissing.filter((m) => m.name !== "shell").length > 0 && (
        <Card title="Brakujące akcje z runbooków">
          <ul className="act-missing-list">
            {visibleMissing
              .filter((m) => m.name !== "shell")
              .map((m) => (
                <li key={m.name} className="act-missing-item">
                  <div>
                    <Text weight="semibold">{m.name}</Text>
                    <div className="muted act-small">
                      Używana w runbookach: {m.runbook_ids.join(", ")}
                    </div>
                  </div>
                  <Button appearance="primary" onClick={() => handleStartNew(m.name)}>
                    Skonfiguruj akcję
                  </Button>
                </li>
              ))}
          </ul>
        </Card>
      )}

      <div className="act-layout">
        {/* Lewa kolumna: lista akcji */}
        <aside className="act-sidebar" aria-label="Lista akcji">
          {resource.loading && resource.data === null && <Loading />}
          {!resource.loading && actions.length === 0 && <Empty>Brak akcji.</Empty>}
          <ul className="act-list">
            {actions.map((a) => {
              const active = creatingName === null && a.name === selectedName;
              const isBuiltin = a.kind === "builtin";
              return (
                <li key={a.name}>
                  <button
                    type="button"
                    className={`act-item ${active ? "active" : ""}`}
                    onClick={() => handleSelect(a)}
                  >
                    <div className="act-item-head">
                      <span className="act-item-title">{a.name}</span>
                      <span className="act-item-kind">{isBuiltin ? "Wbudowana" : a.http?.method ?? "HTTP"}</span>
                    </div>
                    {a.description && <div className="act-item-desc">{a.description}</div>}
                    <div className="act-item-sub">
                      {a.risk === "high" ? "Wymaga zatwierdzenia" : "Automatyczna (bez zatwierdzenia)"}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>

        {/* Prawa kolumna: edytor lub podgląd */}
        <div className="act-main">
          {creatingName !== null ? (
            <ActionForm
              key={`__new__:${creatingName}`}
              initial={emptyForm(creatingName)}
              isNew={true}
              takenNames={takenNames}
              onSaved={handleSaved}
            />
          ) : selectedAction ? (
            selectedAction.kind === "builtin" ? (
              <Card title={`Akcja wbudowana: ${selectedAction.name}`}>
                <div className="act-builtin-box">
                  <Text as="p">{selectedAction.description ?? "Akcja systemowa wbudowana w AlertHub."}</Text>
                  <Text as="p" size="small" color="color.text.subtle">
                    Ta akcja jest wbudowana na stałe w system i działa automatycznie w runbookach — nie wymaga konfiguracji adresu URL.
                  </Text>
                </div>
              </Card>
            ) : (
              <ActionForm
                key={selectedAction.name}
                initial={fromAction(selectedAction)}
                isNew={false}
                version={selectedAction.version}
                takenNames={takenNames}
                onSaved={handleSaved}
                onDelete={handleDelete}
              />
            )
          ) : (
            <Card>
              <Heading size="small" as="h2">
                Wybierz akcję z listy po lewej stronie
              </Heading>
            </Card>
          )}
        </div>
      </div>
    </section>
  );
}
