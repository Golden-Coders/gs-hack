// Reguły — edytor: tasks.md 15.1. Lista reguł + edytor (kreator drzewa warunków ↔ YAML, odbiorcy, prezentacja,
// „testuj na zdarzeniu”). Reguła wskazuje KTO ma dostać alert; JAK (kanały) wynika z preferencji osoby.
import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { api, type Schemas } from "../api/client";
import RuleEditor from "./rules/RuleEditor";
import type { Rule } from "./rules/model";
import "./Rules.css";

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

  const [rules, setRules] = useState<Rule[] | null>(null);
  const [people, setPeople] = useState<Person[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [schemas, setSchemas] = useState<string[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [r, p, t, m] = await Promise.all([api.GET("/rules"), api.GET("/people"), api.GET("/teams"), api.GET("/mappings")]);
      if (r.error || !r.data) throw new Error("rules");
      setRules(r.data);
      setPeople(p.data?.people ?? []);
      setTeams(t.data ?? []);
      setSchemas(m.data?.map((x) => x.schema) ?? []);
      setLoadError(null);
    } catch {
      setLoadError("Nie udało się pobrać reguł.");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const select = useCallback(
    (next: { rule?: string; creating?: boolean }) => {
      if (dirty && !window.confirm("Masz niezapisane zmiany w regule. Odrzucić je?")) return;
      setDirty(false);
      setNotice(null);
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
      setNotice(`Utworzono regułę „${rule.name}”. Obowiązuje dla kolejnych alertów.`);
      setParams({ rule: rule.id });
    }
  }

  function onDeleted(id: string) {
    setRules((prev) => (prev ?? []).filter((r) => r.id !== id));
    setDirty(false);
    setNotice("Reguła została usunięta.");
    setParams({});
  }

  return (
    <section className="rules-screen">
      <div className="rules-header">
        <h1>Reguły</h1>
        <button type="button" className="primary" onClick={() => select({ creating: true })}>
          + Nowa reguła
        </button>
      </div>
      <p className="muted rules-lead">
        Reguła mówi, które alerty kogo dotyczą i jak mają wyglądać. Zmiany obowiązują od razu, bez restartu.
      </p>
      {loadError && (
        <p className="error-box" role="alert">
          {loadError}{" "}
          <button type="button" onClick={() => void load()}>
            Spróbuj ponownie
          </button>
        </p>
      )}
      {notice && (
        <p className="rules-saved" role="status">
          {notice}
        </p>
      )}

      <div className="rules-layout">
        <div className="rules-list" role="region" aria-label="Lista reguł">
          {rules === null && !loadError && <p className="loading">Ładowanie…</p>}
          {rules?.length === 0 && <p className="empty">Brak reguł. Utwórz pierwszą przyciskiem „Nowa reguła”.</p>}
          <ul>
            {rules?.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  className="rules-item"
                  aria-current={!creating && selectedId === r.id ? "true" : undefined}
                  onClick={() => select({ rule: r.id })}
                >
                  <span className="rules-item-name">{r.name}</span>
                  <span className="rules-item-meta">
                    <span className="mono">{r.id}</span>
                    {r.enabled === false ? <span className="rules-off">wyłączona</span> : <span className="rules-on">włączona</span>}
                  </span>
                  <span className="rules-item-rec muted">→ {recipientsLabel(r, people, teams) || "brak odbiorców"}</span>
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
            <p className="empty">
              {selectedId ? `Nie znaleziono reguły „${selectedId}”. ` : ""}Wybierz regułę z listy albo utwórz nową.
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
