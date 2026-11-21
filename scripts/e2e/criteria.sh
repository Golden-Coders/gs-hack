#!/usr/bin/env bash
# 18.6 — kryteria demo niezależne od UI (część P3/P2):
#  (1) nowa akcja HTTP w < 1 min: runbook `cdn-stale` ma nieznaną akcję `flush_cdn` -> krok manualny
#      i wpis w GET /actions/missing; po POST /actions kolejny run wykonuje krok automatycznie
#      (wywołanie w mock-targets GET /calls), a /actions/missing nie zawiera już flush_cdn.
#  (2) bezpieczeństwo: runbook `shell-demo` (action: shell, command: rm -rf /) niczego nie wykonuje
#      (krok manualny, brak nowych wywołań w /calls); akcja z hostem spoza allowlisty kończy krok
#      błędem „host niedozwolony” bez wywołania.
#  (3) zmiana reguły bez restartu (tylko RUN_RULE_CHANGE=1): PUT /rules/settlements-prod z dodatkowym
#      odbiorcą (kasia) -> kolejne zdarzenie ma dostarczenie dla kasia; reguła wraca do pierwotnej treści.
# Wymaga plików config/runbooks/{cdn-stale,shell-demo}.yaml (seed). Sprząta w trap: akcja flush_cdn,
# reguła (jeśli była zmieniona).
# Użycie: API_URL=http://localhost:8088 [RUN_RULE_CHANGE=1] scripts/e2e/criteria.sh
# Zmienne: API_URL, INGEST_URL, MOCK_URL, API_TOKEN, PERSON_ID, TIMEOUT_SECS, LIMIT_SECS (limit kroku (1), domyślnie 60).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
API_URL="${API_URL:-http://localhost:${API_PORT:-8080}}"
INGEST_URL="${INGEST_URL:-http://localhost:8081}"
MOCK_URL="${MOCK_URL:-http://localhost:8090}"
PERSON="${PERSON_ID:-anna}"
# Sesja operatora (POST /auth/login); API_TOKEN nadpisuje, gdy ustawiony.
TOKEN="${API_TOKEN:-$(curl -sS -X POST "$API_URL/auth/login" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$PERSON\",\"password\":\"${PERSON_PASSWORD:-haslo}\"}" | jq -r '.session_token // empty')}"
TIMEOUT="${TIMEOUT_SECS:-45}"
LIMIT="${LIMIT_SECS:-60}"
FIXTURE="$ROOT/crates/contracts/fixtures/events/workflow_failed.json"
RUN="$(date +%s)$RANDOM"
RULE_ID=settlements-prod
RULE_BACKUP=""

STAGE="start"
fail() { echo "FAIL [$STAGE]: $*" >&2; exit 1; }
ok() { echo "OK   [$STAGE] $*"; }
for bin in curl jq; do command -v "$bin" >/dev/null || fail "brak programu $bin"; done

api() { curl -sS -f -X "$1" -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" "${@:3}" "$2"; }
json_api() { api "$1" "$2" -H 'Content-Type: application/json' --data-binary "$3"; }
cleanup() {
    api DELETE "$API_URL/actions/flush_cdn" >/dev/null 2>&1 || true
    [ -z "$RULE_BACKUP" ] || json_api PUT "$API_URL/rules/$RULE_ID" "$RULE_BACKUP" >/dev/null 2>&1 || echo "UWAGA: nie udało się przywrócić reguły $RULE_ID" >&2
}
trap cleanup EXIT
wait_for() {
    local what="$1"; shift
    local deadline=$((SECONDS + TIMEOUT)) out
    while :; do
        if out="$("$@" 2>/dev/null)" && [ -n "$out" ]; then printf '%s' "$out"; return 0; fi
        [ "$SECONDS" -lt "$deadline" ] || fail "po ${TIMEOUT}s brak: $what"
        sleep 1
    done
}
# ingest <type> <error.code> [workflowName] -> POST /ingest; fingerprint workflow.v1 = workflowName|step|error.code
ingest() {
    local env code
    env="$(jq -c --arg t "$1" --arg id "$2" --arg wf "${3:-order-settlement}" \
        '.payload.type = $t | .payload.error.code = $id | .payload.step = $id | .payload.workflowId = $id
         | .payload.workflowName = $wf | .payload.timestamp = (now | todate)' "$FIXTURE")"
    code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$INGEST_URL/ingest" -H 'Content-Type: application/json' --data-binary "$env")" \
        || fail "ingest niedostępny"
    [ "$code" = "202" ] || fail "POST /ingest zwrócił $code"
}
alert_id() { api GET "$API_URL/alerts?limit=1000" | jq -r --arg id "$1" '[.[] | select(.fingerprint | contains($id))] | first | .id // empty'; }
# step_of <alert_id> <runbook_id> -> JSON pierwszego kroku runa
step_of() { api GET "$API_URL/alerts/$1/runbooks" | jq -ce --arg r "$2" '[.[] | select(.runbook_id == $r)] | first // empty | . as $rb | .steps[0] + {run_id: $rb.run_id}'; }
calls() { curl -sS -f "$MOCK_URL/calls"; }
missing() { api GET "$API_URL/actions/missing" | jq -r '.[].name'; }

wait_for "$API_URL/health" api GET "$API_URL/health" >/dev/null
api GET "$MOCK_URL/calls" >/dev/null 2>&1 || curl -sS -f "$MOCK_URL/calls" >/dev/null || fail "mock-targets $MOCK_URL niedostępne"

# ------------------------------------------------------------ (1) nowa akcja HTTP < 1 min
STAGE="new-action-before"
api DELETE "$API_URL/actions/flush_cdn" >/dev/null 2>&1 || true
is_missing() { missing | grep -x flush_cdn; }
wait_for "flush_cdn w /actions/missing (brak seedu config/runbooks/cdn-stale.yaml?)" bash -c "$(declare -f api missing is_missing); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; is_missing" >/dev/null
ingest CDN_STALE "CDN1-$RUN"
A1="$(wait_for "alert CDN1-$RUN" alert_id "CDN1-$RUN")"
manual_step() { step_of "$A1" cdn-stale | jq -ce 'select(.status == "waiting_manual" and .automation == "manual")'; }
wait_for "krok flush_cdn jako manualny (nieznana akcja)" manual_step >/dev/null
wait_for "flush_cdn w GET /actions/missing" bash -c "$(declare -f api missing); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; missing | grep -x flush_cdn" >/dev/null
ok "bez akcji: krok manualny, flush_cdn w /actions/missing"

STAGE="new-action"
T0=$SECONDS
json_api POST "$API_URL/actions" '{"name":"flush_cdn","kind":"http","risk":"low","description":"Flush CDN (e2e)",
    "http":{"method":"POST","url":"http://mock-targets:8090/flush_cdn","headers":{},"body":null,"timeout_ms":5000}}' >/dev/null
# rejestr akcji przeładowuje się w runbooks cyklicznie (~2 s): akcja jest „przyjęta”, gdy znika z /actions/missing
not_missing() { ! missing | grep -qx flush_cdn && echo yes; }
wait_for "przeładowanie rejestru (flush_cdn znika z /actions/missing)" bash -c "$(declare -f api missing not_missing); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; not_missing" >/dev/null
ingest CDN_STALE "CDN2-$RUN"
A2="$(wait_for "alert CDN2-$RUN" alert_id "CDN2-$RUN")"
auto_step() { step_of "$A2" cdn-stale | jq -ce 'select(.automation == "automated" and .status == "succeeded")'; }
S="$(wait_for "krok flush_cdn wykonany automatycznie" auto_step)"
ELAPSED=$((SECONDS - T0))
[ "$ELAPSED" -lt "$LIMIT" ] || fail "nowa akcja zadziałała po ${ELAPSED}s (limit ${LIMIT}s)"
RID="$(jq -r .run_id <<<"$S")"
calls | jq -e --arg r "$RID" 'any(.[]; .path == "/flush_cdn" and (.idempotency_key // "" | startswith("step:" + $r)))' >/dev/null \
    || fail "brak wywołania /flush_cdn dla runa $RID w GET /calls"
! missing | grep -qx flush_cdn || fail "flush_cdn nadal w /actions/missing"
ok "akcja dodana i wykonana w ${ELAPSED}s (< ${LIMIT}s), /actions/missing bez flush_cdn"

# ------------------------------------------------------------ (2) bezpieczeństwo
STAGE="shell-step"
BEFORE="$(calls | jq length)"
ingest SHELL_DEMO "SH-$RUN"
A3="$(wait_for "alert SH-$RUN" alert_id "SH-$RUN")"
sh_step() { step_of "$A3" shell-demo | jq -ce 'select(.status == "waiting_manual" and .automation == "manual")'; }
wait_for "krok shell jako manualny" sh_step >/dev/null
sleep 3
[ "$(calls | jq length)" = "$BEFORE" ] || fail "krok shell wywołał coś w mock-targets (zmiana liczby /calls)"
ok "shell: krok manualny, brak wywołań w /calls"

STAGE="allowlist"
# ta sama nazwa akcji, ale host spoza allowlisty (POST/PUT /actions go przyjmuje, wykonanie nie)
json_api PUT "$API_URL/actions/flush_cdn" '{"name":"flush_cdn","kind":"http","risk":"low","description":"zły host (e2e)",
    "http":{"method":"POST","url":"http://evil.example.invalid/flush_cdn","headers":{},"body":null,"timeout_ms":2000}}' >/dev/null
sleep 3 # przeładowanie rejestru (zmieniony host)
BEFORE="$(calls | jq length)"
ingest CDN_STALE "CDN3-$RUN"
A4="$(wait_for "alert CDN3-$RUN" alert_id "CDN3-$RUN")"
bad_step() { step_of "$A4" cdn-stale | jq -ce 'select(.status == "failed")'; }
F="$(wait_for "krok z hostem spoza allowlisty jako failed" bad_step)"
jq -e '.result | test("host|allowlist"; "i")' <<<"$F" >/dev/null || fail "krok failed bez informacji o hoście: $F"
[ "$(calls | jq length)" = "$BEFORE" ] || fail "zmieniła się liczba /calls"
ok "host spoza allowlisty: krok failed (\"$(jq -r .result <<<"$F")\"), bez wywołania"
api DELETE "$API_URL/actions/flush_cdn" >/dev/null

# ------------------------------------------------------------ (3) zmiana reguły bez restartu
if [ "${RUN_RULE_CHANGE:-0}" = "1" ]; then
    STAGE="rule-change"
    RULE_BACKUP="$(api GET "$API_URL/rules/$RULE_ID")"
    json_api PUT "$API_URL/rules/$RULE_ID" "$(jq -c '.recipients += [{type: "person", id: "kasia"}]' <<<"$RULE_BACKUP")" >/dev/null
    ingest WORKFLOW_FAILED "RULE-$RUN"
    A5="$(wait_for "alert RULE-$RUN" alert_id "RULE-$RUN")"
    kasia() { api GET "$API_URL/alerts/$A5/deliveries" | jq -ce '[.[] | select(.person_id == "kasia")] | select(length > 0)'; }
    wait_for "dostarczenie dla kasia po zmianie reguły (bez restartu)" kasia >/dev/null
    ok "po PUT /rules/$RULE_ID nowe zdarzenie ma dostarczenie dla kasia"
    json_api PUT "$API_URL/rules/$RULE_ID" "$RULE_BACKUP" >/dev/null
    RULE_BACKUP=""
    ok "reguła przywrócona"
else
    echo "SKIP [rule-change] krok (3) pominięty (ustaw RUN_RULE_CHANGE=1)"
fi

STAGE="done"
echo "PASS criteria"
