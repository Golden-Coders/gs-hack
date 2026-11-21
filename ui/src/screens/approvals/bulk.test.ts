import { describe, expect, it, vi } from "vitest";
import { failureMessage, pruneSelection, runBulk, stepKey, stepsLabel, type SendFn, type StepRef, type StepResult } from "./bulk";

const steps: StepRef[] = [
  { run_id: "r1", step_id: "s1" },
  { run_id: "r1", step_id: "s2" },
  { run_id: "r2", step_id: "s3" },
];
const sendStatus = (...codes: number[]): SendFn => {
  const q = [...codes];
  return vi.fn(async () => {
    const status = q.shift() ?? 200;
    return { ok: status >= 200 && status < 300, status };
  });
};

describe("runBulk", () => {
  it("wszystkie sukcesy", async () => {
    const send = sendStatus(200, 200, 200);
    const out = await runBulk(steps, "approve", { personId: "p", send });
    expect(out).toMatchObject({ sent: true, okCount: 3, failCount: 0 });
    expect(send).toHaveBeenCalledTimes(3);
    if (out.sent) expect(out.results.map((r) => r.key)).toEqual(["r1:s1", "r1:s2", "r2:s3"]);
  });

  it("częściowa porażka 409 i 404 nie przerywa reszty", async () => {
    const send = sendStatus(409, 200, 404);
    const out = await runBulk(steps, "approve", { personId: "p", send });
    expect(send).toHaveBeenCalledTimes(3);
    expect(out.sent && [out.okCount, out.failCount]).toEqual([1, 2]);
    if (!out.sent) return;
    expect(out.results.map((r) => r.status)).toEqual([409, 200, 404]);
    expect(out.results[0]).toMatchObject({ ok: false, message: failureMessage(409) });
    expect(out.results[1].message).toBeUndefined();
    expect(out.results[2]).toMatchObject({ ok: false, message: failureMessage(404) });
  });

  it("wyjątek z send = brak sieci (status null), reszta idzie dalej", async () => {
    const send = vi
      .fn<SendFn>()
      .mockResolvedValueOnce({ ok: true, status: 200 })
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({ ok: true, status: 200 });
    const out = await runBulk(steps, "approve", { personId: "p", send });
    expect(send).toHaveBeenCalledTimes(3);
    if (!out.sent) throw new Error("expected sent");
    expect(out.results[1]).toEqual({ key: "r1:s2", ok: false, status: null, message: failureMessage(null) });
    expect([out.okCount, out.failCount]).toEqual([2, 1]);
  });

  it("kolejność sprawdzeń: empty → no-identity → needs-confirmation", async () => {
    const send = sendStatus();
    expect(await runBulk([], "reject", { personId: null, send })).toEqual({ sent: false, reason: "empty" });
    expect(await runBulk(steps, "reject", { personId: null, send })).toEqual({ sent: false, reason: "no-identity" });
    expect(await runBulk(steps, "reject", { personId: "", send })).toEqual({ sent: false, reason: "no-identity" });
    expect(await runBulk(steps, "reject", { personId: "p", send })).toEqual({ sent: false, reason: "needs-confirmation" });
    expect(send).not.toHaveBeenCalled();
  });

  it("reject z confirmed wysyła; approve bez confirmed też", async () => {
    const send = sendStatus();
    await runBulk(steps, "reject", { personId: "p", confirmed: true, send });
    expect(send).toHaveBeenCalledTimes(3);
    expect(send).toHaveBeenCalledWith(steps[0], "reject");
    const send2 = sendStatus();
    const out = await runBulk(steps, "approve", { personId: "p", send: send2 });
    expect(out.sent).toBe(true);
    expect(send2).toHaveBeenCalledWith(steps[0], "approve");
  });

  it("wysyła sekwencyjnie i woła onStep po każdym kroku", async () => {
    const log: string[] = [];
    const send: SendFn = async (s) => {
      log.push(`start ${s.step_id}`);
      await new Promise((r) => setTimeout(r, 5));
      log.push(`end ${s.step_id}`);
      return { ok: true, status: 200 };
    };
    const seen: StepResult[] = [];
    await runBulk(steps, "approve", { personId: "p", send, onStep: (r) => (log.push(`step ${r.key}`), seen.push(r)) });
    expect(log).toEqual([
      "start s1", "end s1", "step r1:s1",
      "start s2", "end s2", "step r1:s2",
      "start s3", "end s3", "step r2:s3",
    ]);
    expect(seen).toHaveLength(3);
  });
});

describe("stepKey / failureMessage / pruneSelection / stepsLabel", () => {
  it("stepKey", () => expect(stepKey(steps[0])).toBe("r1:s1"));

  it("failureMessage: różne komunikaty", () => {
    const msgs = [null, 409, 404, 500].map(failureMessage);
    expect(new Set(msgs).size).toBe(4);
    expect(failureMessage(500)).toContain("500");
  });

  it("pruneSelection zostawia tylko widoczne klucze i nie mutuje wejścia", () => {
    const sel = new Set(["a", "b", "c"]);
    expect([...pruneSelection(sel, ["b", "c", "d"])]).toEqual(["b", "c"]);
    expect(sel.size).toBe(3);
    expect(pruneSelection(sel, []).size).toBe(0);
  });

  it.each([
    [1, "1 krok"], [2, "2 kroki"], [5, "5 kroków"], [12, "12 kroków"], [22, "22 kroki"], [0, "0 kroków"],
  ])("stepsLabel(%i)", (n, expected) => expect(stepsLabel(n)).toBe(expected));
});
