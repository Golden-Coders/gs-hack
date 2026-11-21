#!/usr/bin/env python3
"""Schodkowy test obciążenia: szuka maksymalnej utrzymywalnej przepustowości i wąskich gardeł.

Dla każdego tempa z --rates:
  1. uruchamia N równoległych kontenerów `loadgen` (w sieci compose, bez narzutu portów Docker Desktop),
  2. próbkuje co ~2 s: lag i offsety grup konsumenckich Kafki (rpk), CPU/RAM kontenerów (docker stats),
  3. po zakończeniu ruchu czeka na opróżnienie kolejek (drain) i zbiera p50/p99 e2e z Postgresa,
  4. klasyfikuje krok jako SUSTAINED / DEGRADED / FAILED.
Po pierwszym kroku niespełniającym kryteriów robi jeszcze jeden (potwierdzenie) i kończy.

Użycie (stos `gsload` już działa, patrz loadtest/README.md):
  loadtest/step_load.py --rates 200,500,1000,2000,4000 --step-secs 30
Wynik: tabela na stdout + JSON w loadtest/results/.
"""
from concurrent.futures import ThreadPoolExecutor
import argparse, json, math, os, re, subprocess, sys, threading, time
from datetime import datetime, timezone

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
GROUPS = ["lifecycle-normalizer", "lifecycle-dedup", "router", "runbooks", "sender", "sender-retry", "ingest-source-trades"]


def sh(cmd, check=True, timeout=120):
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: {r.stderr.strip()}")
    return r.stdout


class Env:
    def __init__(self, project):
        self.project = project
        self.compose = ["docker", "compose", "-p", project, "-f", os.path.join(ROOT, "docker-compose.yml"),
                        "-f", os.path.join(ROOT, "loadtest/docker-compose.loadtest.yml")]
        self.pg = f"{project}-postgres-1"
        self.rp = f"{project}-redpanda-1"

    def psql(self, sql):
        return sh(["docker", "exec", self.pg, "psql", "-U", "alerting", "-d", "alerting", "-tA", "-F", "|", "-c", sql]).strip()

    def _group(self, g):
        out = sh(["docker", "exec", self.rp, "rpk", "group", "describe", g, "-c"], check=False)
        lag = committed = logend = 0
        for line in out.splitlines()[1:]:
            c = line.split()
            if len(c) < 6 or not c[1].isdigit():
                continue
            num = lambda x: int(x) if x.isdigit() else 0
            committed += num(c[2]); logend += num(c[4]); lag += num(c[5])
        return g, (lag, committed, logend)

    def lag(self):
        """{grupa: (suma_lag, suma_committed, suma_log_end)} z `rpk group describe -c` (równolegle)."""
        with ThreadPoolExecutor(len(GROUPS)) as ex:
            return dict(ex.map(self._group, GROUPS))

    def stats(self):
        out = sh(["docker", "stats", "--no-stream", "--format", "{{json .}}"], check=False)
        res = {}
        for line in out.splitlines():
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not d["Name"].startswith(self.project + "-"):
                continue
            name = d["Name"][len(self.project) + 1:]
            mem = d["MemUsage"].split("/")[0].strip()
            res[name] = (float(d["CPUPerc"].rstrip("%")), mem)
        return res


def pct(vals, p):
    if not vals:
        return None
    vals = sorted(vals)
    return vals[min(len(vals) - 1, int(math.ceil(p / 100 * len(vals))) - 1)]


def settle(env, timeout):
    """Czeka na pusty system (lag=0), żeby zaległości poprzedniego kroku nie zafałszowały następnego."""
    t = time.time()
    while time.time() - t < timeout:
        if sum(v[0] for v in env.lag().values()) == 0:
            return True
        time.sleep(2)
    return False


def run_step(env, rate, secs, loadgens, dup_ratio, mode, drain_timeout):
    if not settle(env, 600):
        print("  UWAGA: system nie opróżnił się przed krokiem (wyniki mogą być zaniżone)", file=sys.stderr)
    t0 = time.time()
    since = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S.%f+00")
    base_events = int(env.psql("select coalesce(sum(count),0) from alerts") or 0)
    base_alerts = int(env.psql("select count(*) from alerts") or 0)
    base_deliv = int(env.psql("select count(*) from deliveries") or 0)

    samples = []  # (t, {grupa:(lag,committed)}, stats)
    stop = threading.Event()

    def sampler():
        while not stop.is_set():
            ts = time.time()
            try:
                samples.append((ts, env.lag(), env.stats()))
            except Exception as e:  # sampling nie może wywrócić testu
                print(f"  [sampler] {e}", file=sys.stderr)
            stop.wait(max(0.0, 1.0))

    th = threading.Thread(target=sampler, daemon=True)
    th.start()

    per = max(1, rate // loadgens)
    procs = []
    for i in range(loadgens):
        cmd = env.compose + ["run", "--rm", "-T", "--no-deps", "-e", "INGEST_URL=http://ingest:8081", "-e", f"RATE={per}",
                             "-e", f"DURATION_SECS={secs}", "-e", f"DUP_RATIO={dup_ratio}", "-e", f"MODE={mode}",
                             "-e", f"LOADGEN_ID={i}", "loadgen"]
        procs.append(subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True))
    outs = [p.communicate()[0] for p in procs]
    t_load_end = time.time()

    sent = errors = 0
    for o in outs:
        m = re.search(r"wysłane: (\d+)", o); sent += int(m.group(1)) if m else 0
        m = re.search(r"błędy: (\d+)", o); errors += int(m.group(1)) if m else 0
    load_span = secs  # loadgen mierzy własny czas; tempo liczymy względem zadanej długości
    achieved = sent / load_span

    lag_end_load = sum(v[0] for v in (samples[-1][1] if samples else {}).values())
    # drain: czekamy aż lag = 0 oraz wszystkie zdarzenia zliczone przez dedup
    t_drain0 = time.time()
    drained = False
    counted = base_events
    while time.time() - t_drain0 < drain_timeout:
        lg = samples[-1][1] if samples else {}
        counted = int(env.psql("select coalesce(sum(count),0) from alerts") or 0)
        if lg and sum(v[0] for v in lg.values()) == 0 and counted - base_events >= sent:
            drained = True
            break
        time.sleep(1)
    drain_secs = time.time() - t_drain0
    stop.set(); th.join(timeout=5)

    # --- agregaty z próbek podczas ruchu ---
    load_samples = [s for s in samples if t0 <= s[0] <= t_load_end]
    max_lag = {}
    for _, lg, _ in load_samples:
        for g, (lag, *_) in lg.items():
            max_lag[g] = max(max_lag.get(g, 0), lag)
    # przepustowość każdej grupy w czasie ruchu: Δ committed offset / Δt (środkowe 80% okna, bez rozbiegu)
    rates = {}
    if len(load_samples) >= 4:
        a, b = load_samples[len(load_samples) // 5], load_samples[-1]
        dt = max(b[0] - a[0], 1e-9)
        for g in GROUPS:
            if g in a[1] and g in b[1]:
                rates[g] = (b[1][g][1] - a[1][g][1]) / dt
    cpu = {}
    for _, _, st in load_samples:
        for n, (c, _) in st.items():
            cpu.setdefault(n, []).append(c)
    cpu_stat = {n: {"avg": sum(v) / len(v), "max": max(v)} for n, v in cpu.items()}
    mem = {n: st for n, st in (load_samples[-1][2].items() if load_samples else [])}

    new_alerts = int(env.psql("select count(*) from alerts") or 0) - base_alerts
    deliveries = int(env.psql("select count(*) from deliveries") or 0) - base_deliv
    lat = env.psql(
        "select channel, count(*), "
        "round((percentile_cont(0.5) within group (order by extract(epoch from (delivered_at-received_at))*1000))::numeric,1), "
        "round((percentile_cont(0.99) within group (order by extract(epoch from (delivered_at-received_at))*1000))::numeric,1) "
        f"from deliveries where status='delivered' and delivered_at is not null and received_at >= '{since}' group by channel")
    latency = {}
    for line in lat.splitlines():
        ch, n, p50, p99 = line.split("|")
        latency[ch] = {"n": int(n), "p50_ms": float(p50), "p99_ms": float(p99)}
    pg_conn = env.psql("select count(*) from pg_stat_activity where datname='alerting'")

    backlog_secs = lag_end_load / max(rate, 1)
    ok_rate = achieved >= 0.95 * rate and errors <= 0.01 * max(sent, 1)
    if ok_rate and backlog_secs <= 3 and drained and drain_secs <= 30:
        verdict = "SUSTAINED"
    elif ok_rate and drained:
        verdict = "DEGRADED"      # ruch przyjęty, ale przetwarzanie nie nadąża (rośnie kolejka)
    else:
        verdict = "FAILED"

    return {
        "target_rate": rate, "loadgens": loadgens, "secs": secs, "sent": sent, "errors": errors,
        "achieved_rate": round(achieved, 1), "lag_at_end_of_load": lag_end_load, "backlog_secs": round(backlog_secs, 2),
        "drained": drained, "drain_secs": round(drain_secs, 1),
        "events_counted": counted - base_events, "new_alerts": new_alerts, "deliveries": deliveries,
        "e2e_processed_rate": round((counted - base_events) / max(time.time() - t0 - (0 if drained else 0) - 0, 1e-9), 1),
        "max_lag": max_lag, "stage_rate_per_s": {g: round(v, 1) for g, v in rates.items()},
        "cpu_pct": {n: {k: round(x, 1) for k, x in d.items()} for n, d in cpu_stat.items()},
        "mem": {n: m[1] for n, m in mem.items()}, "latency": latency, "pg_connections": pg_conn, "verdict": verdict,
        "t_start": t0,
    }


def print_step(r):
    print(f"\n=== {r['target_rate']} msg/s (x{r['loadgens']} loadgen) -> {r['verdict']}")
    print(f"  wysłane {r['sent']}, błędy {r['errors']}, osiągnięte {r['achieved_rate']} msg/s; "
          f"lag na końcu ruchu {r['lag_at_end_of_load']} ({r['backlog_secs']} s zaległości); drain {r['drain_secs']} s (ok={r['drained']})")
    print(f"  zliczone zdarzenia {r['events_counted']}, nowe alerty {r['new_alerts']}, dostawy {r['deliveries']}, połączenia PG {r['pg_connections']}")
    print("  max lag:   " + ", ".join(f"{g}={v}" for g, v in r["max_lag"].items()))
    print("  przepustowość etapów [msg/s]: " + ", ".join(f"{g}={v}" for g, v in r["stage_rate_per_s"].items()))
    top = sorted(r["cpu_pct"].items(), key=lambda kv: -kv[1]["avg"])[:6]
    print("  CPU avg/max [%]: " + ", ".join(f"{n}={d['avg']:.0f}/{d['max']:.0f}" for n, d in top))
    for ch, d in r["latency"].items():
        print(f"  e2e {ch}: n={d['n']} p50={d['p50_ms']} ms p99={d['p99_ms']} ms")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", default="gsload")
    ap.add_argument("--rates", default="200,500,1000,2000,4000")
    ap.add_argument("--step-secs", type=int, default=30)
    ap.add_argument("--per-loadgen", type=int, default=2500, help="maks. tempo na jedną instancję loadgen")
    ap.add_argument("--dup-ratio", default="0.5")
    ap.add_argument("--mode", default="both")
    ap.add_argument("--drain-timeout", type=int, default=180)
    ap.add_argument("--no-stop", action="store_true", help="nie kończ po pierwszym kroku niespełniającym kryteriów")
    ap.add_argument("--label", default="run")
    a = ap.parse_args()

    env = Env(a.project)
    results, failures = [], 0
    for rate in [int(x) for x in a.rates.split(",")]:
        n = max(1, math.ceil(rate / a.per_loadgen))
        print(f"\n>>> krok {rate} msg/s, {a.step_secs} s, {n} loadgen(ów)", flush=True)
        r = run_step(env, rate, a.step_secs, n, a.dup_ratio, a.mode, a.drain_timeout)
        results.append(r)
        print_step(r)
        if r["verdict"] != "SUSTAINED":
            failures += 1
            if failures >= 2 and not a.no_stop:
                break
        time.sleep(5)

    sustained = [r["target_rate"] for r in results if r["verdict"] == "SUSTAINED"]
    print(f"\nMaks. utrzymywalne tempo (SUSTAINED): {max(sustained) if sustained else 'brak'} msg/s")
    os.makedirs(os.path.join(ROOT, "loadtest/results"), exist_ok=True)
    out = os.path.join(ROOT, f"loadtest/results/{a.label}-{datetime.now().strftime('%Y%m%d-%H%M%S')}.json")
    json.dump({"args": vars(a), "steps": results}, open(out, "w"), indent=2, ensure_ascii=False)
    print("Zapisano:", out)


if __name__ == "__main__":
    main()
