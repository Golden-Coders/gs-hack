// Reguły — edytor: tasks.md 15.1. Lista reguł + edytor (kreator drzewa warunków ↔ YAML, odbiorcy, prezentacja,
// „testuj na zdarzeniu”). Reguła wskazuje KTO ma dostać alert; JAK (kanały) wynika z preferencji osoby.
import { useCallback, useEffect, useState } from "react";
import Button from "@atlaskit/button/new";
import { useFlags } from "@atlaskit/flag";
import { Text } from "@atlaskit/primitives/compiled";
import Code from "@atlaskit/code/inline";
import { Plus } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import { useResource } from "../api/useResource";
import { adsIcon } from "../ui/icons";
import { Empty, ErrorBox, Loading, PageHeader, StatusLozenge } from "../ui/primitives";
import RuleEditor from "./rules/RuleEditor";
import type { Rule } from "./rules/model";
import "./Rules.css";

const PlusIcon = adsIcon(Plus);

type Person = Schemas["PersonDef"];
type Team = Schemas["TeamDef"];

function recipientsLabel(rule: Rule, people: Person[], teams: Team[]): string {
  return rule.recipients
    .map((r) => (r.type === "team" ? teams.find((t) => t.id === r.id)?.name : people.find((p) => p.id === r.id)?.name) ?? r.id)
    .join(", ");
}

export default function Rules() {
  const [params, setParams] = useSearchParams();
  const selectedId = params.get("rule");
  const creating = params.get("new") === "1";

  // Lista reguł żyje lokalnie (zapis i usunięcie aktualizują ją od razu); useResource dostarcza pierwsze ładowanie.
  const resource = useResource(async () => {
    const [r, p, t, m] = await Promise.all([api.GET("/rules"), api.GET("/people"), api.GET("/teams"), api.GET("/mappings")]);
    if (r.data === undefined) return { error: r.error, response: r.response };
    return {
      data: {
        rules: r.data,
        people: p.data?.people ?? [],
        teams: t.data ?? [],
        schemas: m.data?.map((x) => x.schema) ?? [],
      },
      response: r.response,
    };
  });
  const [rules, setRules] = useState<Rule[] | null>(null);
  const [dirty, setDirty] = useState(false);
  const { showFlag } = useFlags();

  useEffect(() => {
    if (resource.data) setRules(resource.data.rules);
  }, [resource.data]);

  const people = resource.data?.people ?? [];
  const teams = resource.data?.teams ?? [];
  const schemas = resource.data?.schemas ?? [];

  const select = useCallback(
    (next: { rule?: string; creating?: boolean }) => {
      if (dirty && !window.confirm("Masz niezapisane zmiany w regule. Odrzucić je?")) return;
      setDirty(false);
      setParams(next.creating ? { new: "1" } : next.rule ? { rule: next.rule } : {});
    },
    [dirty, setParams],
  );

  const selected = !creating && selectedId ? rules?.find((r) => r.id === selectedId) : undefined;

  function onSaved(rule: Rule, wasNew: boolean) {
    setRules((prev) => {
      const list = prev ?? [];
      return list.some((r) => r.id === rule.id) ? list.map((r) => (r.id === rule.id ? rule : r)) : [...list, rule];
    });
    if (wasNew) {
      setDirty(false);
      showFlag({ title: `Utworzono regułę „${rule.name}”`, description: "Obowiązuje dla kolejnych alertów.", appearance: "success", isAutoDismiss: true });
      setParams({ rule: rule.id });
    }
  }

  function onDeleted(id: string) {
    setRules((prev) => (prev ?? []).filter((r) => r.id !== id));
    setDirty(false);
    showFlag({ title: "Reguła została usunięta", appearance: "success", isAutoDismiss: true });
    setParams({});
  }

  return (
    <section className="rules-screen">
      <PageHeader
        actions={
          <Button appearance="primary" iconBefore={PlusIcon} onClick={() => select({ creating: true })}>
            Nowa reguła
          </Button>
        }
      >
        Reguły
      </PageHeader>
      <Text as="p" color="color.text.subtle">
        Reguła mówi, które alerty kogo dotyczą i jak mają wyglądać. Zmiany obowiązują od razu, bez restartu.
      </Text>
      {resource.error && <ErrorBox message={resource.error} onRetry={resource.reload} disabled={resource.loading} />}

      <div className="rules-layout">
        <div className="rules-list" role="region" aria-label="Lista reguł">
          {rules === null && !resource.error && <Loading />}
          {rules?.length === 0 && <Empty>Brak reguł. Utwórz pierwszą przyciskiem „Nowa reguła”.</Empty>}
          <ul>
            {rules?.map((r) => (
              <li key={r.id}>
                {/* eslint-disable-next-line @atlaskit/design-system/no-html-button -- wiersz wyboru z wielolinijkową zawartością; Pressable wymaga xcss, a styl wiersza jest w Rules.css */}
                <button
                  type="button"
                  className="rules-item"
                  aria-current={!creating && selectedId === r.id ? "true" : undefined}
                  onClick={() => select({ rule: r.id })}
                >
                  <span className="rules-item-name">{r.name}</span>
                  <span className="rules-item-meta">
                    <Code>{r.id}</Code>
                    <StatusLozenge domain="rule" value={r.enabled !== false} />
                  </span>
                  <Text size="small" color="color.text.subtle">
                    → {recipientsLabel(r, people, teams) || "brak odbiorców"}
                  </Text>
                </button>
              </li>
            ))}
          </ul>
        </div>

        <div className="rules-main">
          {creating && <RuleEditor key="new" initial={null} people={people} teams={teams} schemas={schemas} onSaved={onSaved} onDeleted={onDeleted} onDirtyChange={setDirty} />}
          {!creating && selected && (
            <RuleEditor key={selected.id} initial={selected} people={people} teams={teams} schemas={schemas} onSaved={onSaved} onDeleted={onDeleted} onDirtyChange={setDirty} />
          )}
          {!creating && !selected && rules !== null && (
            <Empty>
              {selectedId ? `Nie znaleziono reguły „${selectedId}”. ` : ""}Wybierz regułę z listy albo utwórz nową.
            </Empty>
          )}
        </div>
      </div>
    </section>
  );
}
