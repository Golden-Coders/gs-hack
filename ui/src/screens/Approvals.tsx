// Kolejka zatwierdzeń — tasks.md 14.3.
// Zatwierdzać i odrzucać można tylko tutaj (z UI); autorem jest wybrana tożsamość („działam jako”).
import { Text } from "@atlaskit/primitives/compiled";
import { useRef, useState } from "react";
import Badge from "@atlaskit/badge";
import Button from "@atlaskit/button/new";
import { Code } from "@atlaskit/code";
import { useFlags } from "@atlaskit/flag";
import ModalDialog, { ModalBody, ModalFooter, ModalHeader, ModalTitle, ModalTransition } from "@atlaskit/modal-dialog";
import { Checkbox } from "@atlaskit/checkbox";
import { Inline } from "@atlaskit/primitives/compiled";
import Tooltip from "@atlaskit/tooltip";
import { Link, useNavigate } from "react-router-dom";
import { api, asPerson, type Schemas } from "../api/client";
import { usePersonId } from "../api/session";
import { usePollWhenQuiet, useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { RiskBadge } from "../components/Badges";
import { formatTime } from "../ui/format";
import { useListKeyboard } from "../ui/keyboard";
import { EmptyList, ErrorBox, ListSkeleton, PageHeader, useRerenderOnMount } from "../ui/primitives";
import { type Decision, pruneSelection, runBulk, stepKey, stepsLabel } from "./approvals/bulk";
import "./Approvals.css";

type Pending = Schemas["PendingApproval"];

// Komenda trafia do runbooków asynchronnie (202) – przez chwilę lista z serwera może jeszcze zawierać krok,
// który właśnie zatwierdziliśmy. Ukrywamy go lokalnie, ale tylko na krótko (gdyby komenda przepadła).
const DECIDED_HIDE_MS = 15_000;

const keyOf = (a: Pending) => stepKey(a);

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

function Params({ params }: { params: Pending["params"] }) {
  const entries = Object.entries(params ?? {});
  if (entries.length === 0) return <Text color="color.text.subtle">brak parametrów</Text>;
  return (
    <dl className="approval-params">
      {entries.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{typeof v === "string" ? v : JSON.stringify(v)}</dd>
        </div>
      ))}
    </dl>
  );
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
  const [decidedAt, setDecidedAt] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, Pending>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [rejecting, setRejecting] = useState<Pending[] | null>(null);
  const { showFlag } = useFlags();
  const navigate = useNavigate();

  // Optymistycznie ukrywamy krok, który właśnie zatwierdziliśmy; odświeżenie potwierdzi stan z serwera.
  const now = Date.now();
  const shown = list.data?.filter((a) => now - (decidedAt[keyOf(a)] ?? -Infinity) > DECIDED_HIDE_MS);
  // Krok z błędem (np. 409) zostaje przy komunikacie, nawet gdy serwer już go nie zwraca — do zamknięcia komunikatu.
  const items = shown ? [...shown, ...Object.values(failed).filter((f) => !list.data?.some((a) => keyOf(a) === keyOf(f)))] : null;

  // Na żywo: zmiana statusu kroku (np. zatwierdzenie z innej karty) odświeża kolejkę.
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
    const done = new Set(outcome.results.filter((r) => r.ok).map((r) => r.key));
    setSelected((cur) => new Set([...cur].filter((k) => !done.has(k))));
    if (outcome.results.some((r) => r.status === 409 || r.status === 404)) list.refresh();
    const verb = command === "approve" ? "Zatwierdzono" : "Odrzucono";
    const alertIds = new Set(steps.map((x) => x.alert_id));
    const alertId = alertIds.size === 1 ? steps[0]?.alert_id : undefined;
    showFlag({
      title: `${verb} ${outcome.okCount} z ${steps.length}`,
      description:
        outcome.failCount > 0
          ? `Nie udało się: ${stepsLabel(outcome.failCount)} — szczegóły przy wierszach.`
          : steps.length === 1
            ? steps[0]?.text
            : undefined,
      appearance: outcome.failCount === 0 ? "success" : outcome.okCount === 0 ? "error" : "warning",
      isAutoDismiss: outcome.failCount === 0,
      actions: alertId ? [{ content: "Otwórz alert", onClick: () => navigate(`/alerts/${alertId}`) }] : undefined,
    });
  }

  const sel = pruneSelection(selected, items?.map(keyOf) ?? []);
  const selectedItems = items?.filter((a) => sel.has(keyOf(a))) ?? [];

  const { activeKey } = useListKeyboard({
    keys: items?.map(keyOf) ?? [],
    enabled: !rejecting,
    onOpen: (key) => {
      const a = items?.find((x) => keyOf(x) === key);
      if (a) navigate(`/alerts/${a.alert_id}`);
    },
    onAction: (action, key) => {
      if (action === "Escape") return setSelected(new Set());
      const a = items?.find((x) => keyOf(x) === key);
      if (!a) return;
      if (action === "x") toggle(key as string);
      else if (action === "a" && !busy.has(key as string)) void decide([a], "approve");
      else if (action === "r" && !busy.has(key as string)) setRejecting([a]);
    },
  });

  function dismissError(key: string) {
    setRowErrors((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => k !== key)));
    setFailed((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => k !== key)));
  }

  function toggle(key: string) {
    setSelected((cur) => {
      const next = new Set(cur);
      if (!next.delete(key)) next.add(key);
      return next;
    });
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

      {selectedItems.length > 0 && (
        <div role="region" aria-label="Akcje zbiorcze" className="approvals-bulk">
          <Inline space="space.100" alignBlock="center" shouldWrap>
            <Badge appearance="primary">{selectedItems.length}</Badge>
            <Text>zaznaczone</Text>
            <Button
              appearance="primary"
              isDisabled={!personId || selectedItems.some((a) => busy.has(keyOf(a)))}
              onClick={() => void decide(selectedItems, "approve")}
            >
              Zatwierdź zaznaczone
            </Button>
            <Button appearance="danger" isDisabled={!personId} onClick={() => setRejecting(selectedItems)}>
              Odrzuć zaznaczone
            </Button>
            <Button appearance="subtle" onClick={() => setSelected(new Set())}>
              Wyczyść zaznaczenie
            </Button>
          </Inline>
        </div>
      )}

      {items && items.length > 0 && (
        <ul className="approvals-list" aria-label="Kroki oczekujące na zatwierdzenie">
          {items.map((a) => {
            const key = keyOf(a);
            const isBusy = busy.has(key);
            return (
              <li
                key={key}
                className={["approval", sel.has(key) && "approval--selected", key === activeKey && "kb-current"].filter(Boolean).join(" ")}
              >
                <Checkbox isChecked={sel.has(key)} onChange={() => toggle(key)} label="Zaznacz" />
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
                    <div className="approval-params-row">
                      <dt>Parametry</dt>
                      <dd>
                        <Params params={a.params} />
                      </dd>
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
