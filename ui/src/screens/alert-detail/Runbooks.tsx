// Runbooki dopasowane do alertu: MANUAL = treść Markdown, AUTOMATIC = kroki ze statusem i komendami.
// Zatwierdzanie (approve/reject) jest tylko w kolejce `/approvals` – tu tylko status i link.
import { useState } from "react";
import { Link } from "react-router-dom";
import { api, asPerson, type Schemas } from "../../api/client";
import { usePersonId } from "../../api/session";
import { StepStatusBadge } from "../../components/Badges";
import Markdown from "../../components/Markdown";
import Section from "./Section";
import { send } from "../../api/request";
import type { Resource } from "../../api/useResource";
import { formatTime } from "../../ui/format";

type Command = "done" | "retry" | "skip" | "manual_mode";

const STEP_TYPE: Record<Schemas["StepState"]["type"], string> = {
  check: "sprawdzenie",
  action: "akcja",
  manual: "ręczny",
  note: "notatka",
};

export default function Runbooks({
  resource,
  onChanged,
}: {
  resource: Resource<Schemas["AlertRunbook"][]>;
  onChanged: () => void;
}) {
  const personId = usePersonId();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});

  const runbooks = resource.data ?? [];

  async function command(run: string, step: string, cmd: Command) {
    const key = `${run}/${step}`;
    const note = notes[key]?.trim();
    setBusy(key);
    setError(null);
    const err = await send(() =>
      api.POST("/runs/{run_id}/steps/{step_id}/{command}", {
        params: { path: { run_id: run, step_id: step, command: cmd }, header: asPerson() },
        body: cmd === "done" && note ? { note } : undefined,
      }),
    );
    setBusy(null);
    if (err) setError(`Komenda „${cmd}” dla kroku ${step} nie powiodła się: ${err}`);
    else onChanged();
  }

  const blocked = busy !== null || !personId;
  const hint = personId ? undefined : "Wybierz osobę w polu „Działam jako”";

  return (
    <Section title="Runbooki" resource={resource}>
      {error && (
        <div className="error-box" role="alert">
          {error}
        </div>
      )}
      {!resource.loading && !resource.error && runbooks.length === 0 && (
        <p className="muted">Żaden runbook nie pasuje do tego alertu.</p>
      )}
      {runbooks.map((rb) => (
        <article key={rb.runbook_id} className="ad-runbook">
          <header>
            <h3>{rb.title}</h3>
            <span className="badge badge-gray">{rb.kind === "automatic" ? "automatyczny" : "ręczny"}</span>
            {rb.episode != null && <span className="muted">epizod #{rb.episode}</span>}
          </header>
          {rb.kind === "manual" && rb.body && <Markdown source={rb.body} />}
          {rb.kind === "automatic" && rb.steps.length === 0 && <p className="muted">Runbook bez kroków.</p>}
          {rb.steps.length > 0 && (
            <ol className="ad-steps">
              {rb.steps.map((s, i) => {
                const runId = rb.run_id;
                const key = `${runId}/${s.step_id}`;
                return (
                  <li key={s.step_id} className={`ad-step ad-step-${s.status}`}>
                    <div className="ad-step-main">
                      <span className="ad-step-no">{i + 1}.</span>
                      <StepStatusBadge status={s.status} />
                      <span className="ad-step-text">{s.text}</span>
                    </div>
                    <div className="ad-step-meta muted">
                      {STEP_TYPE[s.type] ?? s.type}
                      {s.action && (
                        <>
                          {" · "}
                          <code>{s.action}</code>
                        </>
                      )}
                      {s.approval === "required" && " · wymaga zatwierdzenia"}
                      {s.automation === "manual" && s.type === "action" && " · do wykonania ręcznie (akcja spoza rejestru)"}
                      {s.condition && ` · jeśli: ${s.condition}`}
                      {s.attempt > 0 && ` · próba ${s.attempt}`}
                      {` · ${formatTime(s.updated_at)}`}
                    </div>
                    {s.result && (
                      <div className={s.status === "failed" ? "ad-step-result ad-error-text" : "ad-step-result"}>
                        Wynik: {s.result}
                      </div>
                    )}
                    {s.note && <div className="ad-step-result">Notatka: {s.note}</div>}

                    {s.status === "waiting_approval" && (
                      <div className="ad-step-actions">
                        <span>Czeka na zatwierdzenie.</span>
                        <Link to="/approvals">Przejdź do zatwierdzeń</Link>
                      </div>
                    )}
                    {runId && s.status === "failed" && (
                      <div className="ad-step-actions">
                        <button type="button" disabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "retry")}>
                          Ponów
                        </button>
                        <button type="button" disabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "skip")}>
                          Pomiń
                        </button>
                        <button type="button" disabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "manual_mode")}>
                          Tryb ręczny
                        </button>
                        {busy === key && <span className="muted">Wysyłanie…</span>}
                      </div>
                    )}
                    {runId && s.status === "waiting_manual" && (
                      <div className="ad-step-actions">
                        <label className="ad-note">
                          <span className="sr-only">Notatka do kroku {i + 1}</span>
                          <input
                            type="text"
                            placeholder="Notatka (opcjonalnie)"
                            value={notes[key] ?? ""}
                            onChange={(e) => setNotes((n) => ({ ...n, [key]: e.target.value }))}
                            disabled={busy !== null}
                          />
                        </label>
                        <button type="button" className="primary" disabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "done")}>
                          Zrobione
                        </button>
                        <button type="button" disabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "skip")}>
                          Pomiń
                        </button>
                        {busy === key && <span className="muted">Wysyłanie…</span>}
                      </div>
                    )}
                  </li>
                );
              })}
            </ol>
          )}
        </article>
      ))}
    </Section>
  );
}
