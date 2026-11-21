
export type Decision = "approve" | "reject";
export type StepRef = { run_id: string; step_id: string };
export type StepResult = { key: string; ok: boolean; status: number | null; message?: string };

/** Odpowiedź wysyłki: `status` HTTP; wyjątek (brak sieci) rzuca `send`. */
export type SendFn = (step: StepRef, command: Decision) => Promise<{ ok: boolean; status: number }>;

export type BulkOutcome =
  | { sent: false; reason: "no-identity" | "needs-confirmation" | "empty" }
  | { sent: true; results: StepResult[]; okCount: number; failCount: number };

export const stepKey = (s: StepRef) => `${s.run_id}:${s.step_id}`;

/** Komunikat przy wierszu; `status === null` = brak połączenia z API. */
export function failureMessage(status: number | null): string {
  if (status === null) return "Cannot connect to API — decision was not sent.";
  if (status === 409) return "Step is no longer waiting for approval (someone acted earlier or the runbook changed).";
  if (status === 404) return "Step not found — it may have been closed.";
  return `Error ${status} while sending decision.`;
}

/**
 * Wysyła decyzję dla każdego kroku po kolei (błąd jednego nie przerywa reszty).
 * Blokady: brak tożsamości (`X-Person-Id` wymagany przez API) oraz odrzucenie bez potwierdzenia.
 */
export async function runBulk(
  steps: StepRef[],
  command: Decision,
  opts: { personId: string | null; confirmed?: boolean; send: SendFn; onStep?: (r: StepResult) => void },
): Promise<BulkOutcome> {
  if (steps.length === 0) return { sent: false, reason: "empty" };
  if (!opts.personId) return { sent: false, reason: "no-identity" };
  if (command === "reject" && !opts.confirmed) return { sent: false, reason: "needs-confirmation" };

  const results: StepResult[] = [];
  for (const step of steps) {
    let r: StepResult;
    try {
      const { ok, status } = await opts.send(step, command);
      r = ok ? { key: stepKey(step), ok, status } : { key: stepKey(step), ok, status, message: failureMessage(status) };
    } catch {
      r = { key: stepKey(step), ok: false, status: null, message: failureMessage(null) };
    }
    results.push(r);
    opts.onStep?.(r);
  }
  const okCount = results.filter((r) => r.ok).length;
  return { sent: true, results, okCount, failCount: results.length - okCount };
}

/** Zaznaczenie po odświeżeniu listy: zostają tylko klucze, które nadal czekają na decyzję. */
export function pruneSelection(selected: ReadonlySet<string>, visibleKeys: readonly string[]): Set<string> {
  const visible = new Set(visibleKeys);
  return new Set([...selected].filter((k) => visible.has(k)));
}

/** Polska liczba mnoga: 1 krok, 2–4 kroki, 5+ kroków (12–14 → kroków). */
export function stepsLabel(n: number): string {
  return `${n} ${n === 1 ? "step" : "steps"}`;
}
