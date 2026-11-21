import { Text } from "@atlaskit/primitives/compiled";
import { useState } from "react";
import { Link } from "react-router-dom";
import { api, asPerson, type Schemas } from "../../api/client";
import { usePersonId } from "../../api/session";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { StepStatusBadge } from "../../components/Badges";
import Markdown from "../../components/Markdown";
import { Card, Empty, StatusLozenge, TextInput } from "../../ui/primitives";
import { useResultFlag } from "./flags";
import { send } from "../../api/request";
import type { Resource } from "../../api/useResource";

type Command = "done" | "retry" | "skip" | "manual_mode";

export default function Runbooks({
  resource,
  onChanged,
}: {
  resource: Resource<Schemas["AlertRunbook"][]>;
  onChanged: () => void;
}) {
  const personId = usePersonId();
  const [busy, setBusy] = useState<string | null>(null);
  const flag = useResultFlag();
  const [notes, setNotes] = useState<Record<string, string>>({});

  const runbooks = resource.data ?? [];

  async function command(run: string, step: string, cmd: Command) {
    const key = `${run}/${step}`;
    const note = notes[key]?.trim();
    setBusy(`${key}/${cmd}`);
    const err = await send(() =>
      api.POST("/runs/{run_id}/steps/{step_id}/{command}", {
        params: { path: { run_id: run, step_id: step, command: cmd }, header: asPerson() },
        body: cmd === "done" && note ? { note } : undefined,
      }),
    );
    setBusy(null);
    flag(err, `Komenda „${cmd}” dla kroku ${step} wykonana`, `Komenda „${cmd}” dla kroku ${step} nie powiodła się`);
    if (!err) onChanged();
  }

  const blocked = busy !== null || !personId;
  const hint = personId ? undefined : "Wybierz osobę w polu „Działam jako”";

  return (
    <Card title="Runbooki" loading={resource.loading} error={resource.error} onRetry={resource.reload}>
      {!resource.loading && !resource.error && runbooks.length === 0 && (
        <Empty>Żaden runbook nie pasuje do tego alertu.</Empty>
      )}
      {runbooks.map((rb) => (
        <article key={rb.runbook_id} className="ad-runbook">
          <header>
            <Heading size="small" as="h3">{rb.title}</Heading>
            <StatusLozenge domain="runbookKind" value={rb.kind} />
          </header>
          {rb.kind === "manual" && rb.body && <Markdown source={rb.body} />}
          {rb.kind === "automatic" && rb.steps.length === 0 && (
            <Text as="p" color="color.text.subtle">Runbook bez kroków.</Text>
          )}
          {rb.steps.length > 0 && (
            <ol className="ad-steps">
              {rb.steps.map((s, i) => {
                const runId = rb.run_id;
                const key = `${runId}/${s.step_id}`;
                const approvalTarget = runId
                  ? `/approvals?step=${encodeURIComponent(`${runId}:${s.step_id}`)}#approval-${runId}-${s.step_id}`
                  : "/approvals";
                return (
                  <li key={s.step_id} className="ad-step">
                    <div className="ad-step-main">
                      <span className="ad-step-no">{i + 1}.</span>
                      <StepStatusBadge status={s.status} />
                      <span className="ad-step-text">{s.text}</span>
                    </div>
                    {s.note && <div className="ad-step-result">Notatka: {s.note}</div>}

                    {s.status === "waiting_approval" && (
                      <div className="ad-step-actions">
                        <Link to={approvalTarget}>Przejdź do zatwierdzenia →</Link>
                      </div>
                    )}
                    {runId && s.status === "failed" && (
                      <div className="ad-step-actions">
                        <Button spacing="compact" isLoading={busy === `${key}/retry`} isDisabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "retry")}>
                          Ponów
                        </Button>
                        <Button spacing="compact" isLoading={busy === `${key}/skip`} isDisabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "skip")}>
                          Pomiń
                        </Button>
                        <Button spacing="compact" isLoading={busy === `${key}/manual_mode`} isDisabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "manual_mode")}>
                          Tryb ręczny
                        </Button>
                      </div>
                    )}
                    {runId && s.status === "waiting_manual" && (
                      <div className="ad-step-actions">
                        <div className="ad-note">
                          <TextInput
                            aria-label={`Notatka do kroku ${i + 1}`}
                            placeholder="Notatka (opcjonalnie)"
                            value={notes[key] ?? ""}
                            onChange={(e) => setNotes((n) => ({ ...n, [key]: e.currentTarget.value }))}
                            isDisabled={busy !== null}
                          />
                        </div>
                        <Button appearance="primary" spacing="compact" isLoading={busy === `${key}/done`} isDisabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "done")}>
                          Zrobione
                        </Button>
                        <Button spacing="compact" isLoading={busy === `${key}/skip`} isDisabled={blocked} title={hint} onClick={() => command(runId, s.step_id, "skip")}>
                          Pomiń
                        </Button>
                      </div>
                    )}
                  </li>
                );
              })}
            </ol>
          )}
        </article>
      ))}
    </Card>
  );
}
