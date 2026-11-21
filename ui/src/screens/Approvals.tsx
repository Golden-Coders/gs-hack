import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useRef, useState } from "react";
import Button from "@atlaskit/button/new";
import { Code } from "@atlaskit/code";
import { useFlags } from "@atlaskit/flag";
import ModalDialog, { ModalBody, ModalFooter, ModalHeader, ModalTitle, ModalTransition } from "@atlaskit/modal-dialog";
import Tooltip from "@atlaskit/tooltip";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { RiskBadge } from "../components/Badges";
import { formatTime } from "../ui/format";
import { useListKeyboard } from "../ui/keyboard";
import { EmptyList, ErrorBox, ListSkeleton, PageHeader, useRerenderOnMount } from "../ui/primitives";
import { type Decision, runBulk, stepKey, stepsLabel } from "./approvals/bulk";
import "./Approvals.css";

type Pending = Schemas["PendingApproval"];

const DECIDED_HIDE_MS = 15_000;

const keyOf = (a: Pending) => stepKey(a);
const domIdOf = (a: Pending) => `approval-${a.run_id}-${a.step_id}`;

function formatSince(iso: string): string {
  const ms = Date.now() - Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  const min = Math.floor(ms / 60_000);
  const ago =
    min < 1
      ? "przed chwilą"
      : min < 60
        ? `${min} min temu`
        : min < 48 * 60
          ? `${Math.floor(min / 60)} h ${min % 60} min temu`
          : `${Math.floor(min / (24 * 60))} dni temu`;
  return `${ago} (${formatTime(iso)})`;
}

function RejectDialog({ steps, onCancel, onConfirm }: { steps: Pending[]; onCancel: () => void; onConfirm: () => void }) {
  const one = steps.length === 1;
  const cancelRef = useRef<HTMLButtonElement>(null);
  useRerenderOnMount(() => cancelRef.current?.focus());
  return (
    <ModalDialog onClose={onCancel} width="small" autoFocus={cancelRef}>
      <ModalHeader hasCloseButton>
        <ModalTitle appearance="danger">{one ? "Odrzucić ten krok?" : `Odrzucić ${stepsLabel(steps.length)}?`}</ModalTitle>
      </ModalHeader>
      <ModalBody>
        <Text as="p">
          {one
            ? `„${steps[0]?.text}” zostanie odrzucony; runbook nie wykona tego kroku.`
            : `Odrzucenie obejmie ${stepsLabel(steps.length)} z różnych runbooków. Tej decyzji nie da się cofnąć.`}
        </Text>
      </ModalBody>
      <ModalFooter>
        <Button appearance="subtle" ref={cancelRef} onClick={onCancel}>
          Anuluj
        </Button>
        <Button appearance="danger" onClick={onConfirm}>
          {one ? "Odrzuć krok" : `Odrzuć ${stepsLabel(steps.length)}`}
        </Button>
      </ModalFooter>
    </ModalDialog>
  );
}

export default function Approvals() {
  const personId = usePersonId();
  const list = useResource(() => api.GET("/approvals"));
  const [searchParams] = useSearchParams();
  const targetStepKey = searchParams.get("step");
  const scrolledForRef = useRef<string | null>(null);
  const [decidedAt, setDecidedAt] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, Pending>>({});
  const [rejecting, setRejecting] = useState<Pending[] | null>(null);
  const { showFlag } = useFlags();
  const navigate = useNavigate();

  const now = Date.now();
  const shown = list.data?.filter((a) => now - (decidedAt[keyOf(a)] ?? -Infinity) > DECIDED_HIDE_MS);
  const items = shown ? [...shown, ...Object.values(failed).filter((f) => !list.data?.some((a) => keyOf(a) === keyOf(f)))] : null;

  useEffect(() => {
    if (!targetStepKey || !items?.length || scrolledForRef.current === targetStepKey) return;
    const target = items.find((a) => keyOf(a) === targetStepKey);
    if (!target) return;
    const el = document.getElementById(domIdOf(target));
    if (el) {
      scrolledForRef.current = targetStepKey;
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [targetStepKey, items]);

  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "step_status") list.refresh();
  });

  usePollWhenQuiet(list.refresh);

  async function decide(steps: Pending[], command: Decision, confirmed = false) {
    const keys = steps.map(keyOf);
    setBusy((cur) => new Set([...cur, ...keys]));
    const byKey = new Map(steps.map((x) => [keyOf(x), x]));
    setRowErrors((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => !keys.includes(k))));
    setFailed((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => !keys.includes(k))));
    const outcome = await runBulk(steps, command, {
      personId,
      confirmed,
      send: async (step, cmd) => {
        const { response } = await api.POST("/runs/{run_id}/steps/{step_id}/{command}", {
          params: { path: { run_id: step.run_id, step_id: step.step_id, command: cmd }, header: asPerson() },
        });
        return { ok: response.ok, status: response.status };
      },
      onStep: (r) => {
        if (r.ok) setDecidedAt((cur) => ({ ...cur, [r.key]: Date.now() }));
        else {
          setRowErrors((cur) => ({ ...cur, [r.key]: r.message ?? "Błąd" }));
          const step = byKey.get(r.key);
          if (step) setFailed((cur) => ({ ...cur, [r.key]: step }));
        }
      },
    });
    setBusy((cur) => new Set([...cur].filter((k) => !keys.includes(k))));
    if (!outcome.sent) {
      if (outcome.reason === "no-identity") {
        showFlag({ title: "Wybierz tożsamość", description: "Bez „działam jako” API nie przyjmie decyzji.", appearance: "warning", isAutoDismiss: true });
      }
      return;
    }
    if (outcome.results.some((r) => r.status === 409 || r.status === 404)) list.refresh();
    if (outcome.failCount > 0) {
      const verb = command === "approve" ? "Zatwierdzono" : "Odrzucono";
      showFlag({
        title: `${verb} ${outcome.okCount} z ${steps.length}`,
        description: `Nie udało się: ${stepsLabel(outcome.failCount)} — szczegóły przy wierszach.`,
        appearance: outcome.okCount === 0 ? "error" : "warning",
        isAutoDismiss: false,
      });
    }
  }

  const { activeKey } = useListKeyboard({
    keys: items?.map(keyOf) ?? [],
    enabled: !rejecting,
    onOpen: (key) => {
      const a = items?.find((x) => keyOf(x) === key);
      if (a) navigate(`/alerts/${a.alert_id}`);
    },
    onAction: (action, key) => {
      const a = items?.find((x) => keyOf(x) === key);
      if (!a) return;
      if (action === "a" && !busy.has(key as string)) void decide([a], "approve");
      else if (action === "r" && !busy.has(key as string)) setRejecting([a]);
    },
  });

  function dismissError(key: string) {
    setRowErrors((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => k !== key)));
    setFailed((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => k !== key)));
  }

  return (
    <section className="approvals">
      <PageHeader actions={<Button onClick={list.reload}>Odśwież</Button>}>Kolejka zatwierdzeń</PageHeader>
      <Text as="p" color="color.text.subtle">
        Kroki runbooków czekające na decyzję człowieka. Zatwierdzenie jest możliwe tylko tutaj
        {personId ? (
          <>
            {" "}
            — decyzję zapiszemy jako <Text as="strong" weight="bold">{personId}</Text>.
          </>
        ) : (
          "."
        )}
      </Text>

      {!personId && (
        <ErrorBox message="Wybierz tożsamość („działam jako” w panelu bocznym), aby zatwierdzać i odrzucać kroki." />
      )}
      {list.error && (
        <ErrorBox message={`Nie udało się pobrać kolejki zatwierdzeń: ${list.error}`} onRetry={list.reload} disabled={list.loading} />
      )}

      {items === null && !list.error && <ListSkeleton label="Ładowanie kolejki zatwierdzeń…" />}
      {items?.length === 0 && (
        <EmptyList header="Brak kroków do zatwierdzenia" description="Kroki runbooków czekające na człowieka pojawią się tu same." />
      )}

      {items && items.length > 0 && (
        <ul className="approvals-list" aria-label="Kroki oczekujące na zatwierdzenie">
          {items.map((a) => {
            const key = keyOf(a);
            const isBusy = busy.has(key);
            const isTargeted = targetStepKey === key;
            return (
              <li
                id={domIdOf(a)}
                key={key}
                className={[
                  "approval",
                  isTargeted && "approval--selected",
                  (key === activeKey || isTargeted) && "kb-current",
                ]
                  .filter(Boolean)
                  .join(" ")}
              >
                <div className="approval-main">
                  <div className="approval-title">
                    <RiskBadge risk={a.risk} />
                    <Text as="strong" weight="bold">{a.text}</Text>
                  </div>
                  <dl className="approval-meta">
                    <div>
                      <dt>Alert</dt>
                      <dd>
                        <Link to={`/alerts/${a.alert_id}`}>{a.alert_title}</Link>
                      </dd>
                    </div>
                    <div>
                      <dt>Runbook</dt>
                      <dd>{a.runbook_title}</dd>
                    </div>
                    <div>
                      <dt>Akcja</dt>
                      <dd>{a.action ? <Code>{a.action}</Code> : <Text color="color.text.subtle">brak (krok ręczny)</Text>}</dd>
                    </div>
                    <div>
                      <dt>Czeka od</dt>
                      <dd>{formatSince(a.waiting_since)}</dd>
                    </div>
                  </dl>
                  {rowErrors[key] && (
                    <>
                      <ErrorBox message={rowErrors[key]} />
                      <Button appearance="subtle" onClick={() => dismissError(key)}>
                        Zamknij komunikat
                      </Button>
                    </>
                  )}
                </div>
                <div className="approval-actions">
                  <Tooltip content="Zatwierdź (a)">
                    <Button appearance="primary" isLoading={isBusy} isDisabled={!personId} onClick={() => void decide([a], "approve")}>
                      {isBusy ? "Wysyłanie…" : "Zatwierdź"}
                    </Button>
                  </Tooltip>
                  <Tooltip content="Odrzuć (r)">
                    <Button isDisabled={!personId || isBusy} onClick={() => setRejecting([a])}>
                      Odrzuć
                    </Button>
                  </Tooltip>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <ModalTransition>
        {rejecting && (
          <RejectDialog
            steps={rejecting}
            onCancel={() => setRejecting(null)}
            onConfirm={() => {
              setRejecting(null);
              void decide(rejecting, "reject", true);
            }}
          />
        )}
      </ModalTransition>
    </section>
  );
}
