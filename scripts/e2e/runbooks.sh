#!/usr/bin/env bash
# 18.3 — runbooki end-to-end:
#  (a) Settlements: WORKFLOW_FAILED/TIMEOUT -> runbook AUTOMATIC `settlement-timeout`: http_get i retry_workflow
#      succeeded (wywołania w mock-targets GET /calls), `escalate` czeka na zatwierdzenie (GET /approvals)
#      -> POST /runs/{run}/steps/s3/approve -> succeeded, zniknął z kolejki, wpis w audycie, SMS/email do
#      zespołu counterparty-support (Kasia).
#  (b) payments: alertmanager HighErrorRate -> runbook MANUAL: w treści emaila (Mailpit) albo, w godzinach 08-20
#      (preferencja Anny: SMS), link do alertu w SMS (mock outbox) bez pełnej treści runbooka.
#  (c) pricing: POST /ingest/pricing.log.v1 -> alert, runbook MANUAL dołączony (GET /alerts/{id}/runbooks),
#      dostarczenie tylko na kanale ui (pricing = UI only; link w SMS sprawdza (b)).
# Wymaga mappingu `pricing.log.v1` (config/mappings) i runbooka pricing-latency z `matches` po polu `service`.
# Użycie: API_URL=http://localhost:8088 scripts/e2e/runbooks.sh
# Zmienne: API_URL, INGEST_URL (:8081), MOCK_URL (:8090), MAILPIT_URL (:8025), API_TOKEN, PERSON_ID, TIMEOUT_SECS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
API_URL="${API_URL:-http://localhost:${API_PORT:-8080}}"
INGEST_URL="${INGEST_URL:-http://localhost:8081}"
MOCK_URL="${MOCK_URL:-http://localhost:8090}"
MAILPIT_URL="${MAILPIT_URL:-http://localhost:8025}"
PERSON="${PERSON_ID:-anna}"
# Sesja operatora (POST /auth/login); API_TOKEN nadpisuje, gdy ustawiony.
TOKEN="${API_TOKEN:-$(curl -sS -X POST "$API_URL/auth/login" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$PERSON\",\"password\":\"${PERSON_PASSWORD:-haslo}\"}" | jq -r '.session_token // empty')}"
TIMEOUT="${TIMEOUT_SECS:-45}"
FIX="$ROOT/crates/contracts/fixtures/events"
RUN="$(date +%s)$RANDOM"

STAGE="start"
fail() { echo "FAIL [$STAGE]: $*" >&2; exit 1; }
ok() { echo "OK   [$STAGE] $*"; }
for bin in curl jq; do command -v "$bin" >/dev/null || fail "brak programu $bin"; done

api() { curl -sS -f -X "$1" -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" "${@:3}" "$2"; }
wait_for() { # wait_for <opis> <polecenie...>: co 1 s, aż zwróci 0 i niepuste wyjście
    local what="$1"; shift
    local deadline=$((SECONDS + TIMEOUT)) out
    while :; do
        if out="$("$@" 2>/dev/null)" && [ -n "$out" ]; then printf '%s' "$out"; return 0; fi
        [ "$SECONDS" -lt "$deadline" ] || fail "po ${TIMEOUT}s brak: $what"
        sleep 1
    done
}
post_ingest() { # post_ingest <ścieżka> <json>
    local code
    code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$INGEST_URL$1" -H 'Content-Type: application/json' --data-binary "$2")" \
        || fail "ingest $INGEST_URL niedostępny"
    [ "$code" = "202" ] || fail "POST $1 zwrócił $code (oczekiwano 202)"
}
find_alert() { # find_alert <fragment fingerprintu>
    api GET "$API_URL/alerts?limit=1000" | jq -r --arg id "$1" '[.[] | select(.fingerprint | contains($id))] | first | .id // empty'
}
runbooks_of() { api GET "$API_URL/alerts/$1/runbooks"; }

STAGE="health"
wait_for "$API_URL/health" api GET "$API_URL/health" >/dev/null
ok "api odpowiada"

# ---------------------------------------------------------------- (a) Settlements
STAGE="settlements-ingest"
WF="E2E-$RUN"
post_ingest /ingest "$(jq -c --arg id "$WF" '.payload.error.code = "TIMEOUT" | .payload.step = $id | .payload.workflowId = $id
    | .payload.timestamp = (now | todate)' "$FIX/workflow_failed.json")"
STAGE="settlements-alert"
S_ALERT="$(wait_for "alert z $WF" bash -c "$(declare -f api find_alert); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; find_alert '$WF'")"
ok "alert $S_ALERT"

STAGE="settlements-run"
s_run() { runbooks_of "$S_ALERT" | jq -ce '[.[] | select(.runbook_id == "settlement-timeout")] | first // empty | select(.steps | length == 3)
    | select(.steps[0].status == "succeeded" and .steps[1].status == "succeeded" and .steps[2].status == "waiting_approval")'; }
RB="$(wait_for "run settlement-timeout: s1,s2 succeeded, s3 waiting_approval" s_run)"
RUN_ID="$(jq -r .run_id <<<"$RB")"
ok "run $RUN_ID: $(jq -r '[.steps[] | .step_id + "=" + .status] | join(" ")' <<<"$RB")"

STAGE="settlements-calls"
calls="$(curl -sS -f "$MOCK_URL/calls")"
for p in "step:$RUN_ID:s1:0:/counterparty-service/health" "step:$RUN_ID:s2:0:/retry_workflow"; do
    key="${p%:*}"; path="${p##*:}"
    jq -e --arg k "$key" --arg p "$path" 'any(.[]; .idempotency_key == $k and .path == $p and .status == 200)' <<<"$calls" >/dev/null \
        || fail "brak wywołania $path (klucz $key) w GET $MOCK_URL/calls"
done
ok "mock-targets: health check i retry_workflow wywołane"

STAGE="settlements-approval"
in_queue() { api GET "$API_URL/approvals" | jq -ce --arg r "$RUN_ID" '[.[] | select(.run_id == $r and .step_id == "s3")] | first // empty'; }
wait_for "krok s3 w GET /approvals" in_queue >/dev/null
api POST "$API_URL/runs/$RUN_ID/steps/s3/approve" >/dev/null
ok "zatwierdzono s3"
s3_done() { runbooks_of "$S_ALERT" | jq -ce '.[] | select(.run_id == "'"$RUN_ID"'") | .steps[2] | select(.status == "succeeded" or .status == "failed")'; }
wait_for "s3 po zatwierdzeniu w stanie succeeded/failed" s3_done >/dev/null
s3="$(runbooks_of "$S_ALERT" | jq -c '.[] | select(.run_id == "'"$RUN_ID"'") | .steps[2]')"
[ "$(jq -r .status <<<"$s3")" = "succeeded" ] || fail "s3 ma status $(jq -r .status <<<"$s3"): $(jq -r .result <<<"$s3")"
[ -z "$(in_queue)" ] || fail "s3 nadal w kolejce zatwierdzeń"
api GET "$API_URL/alerts/$S_ALERT/audit" | jq -e --arg p "$PERSON" \
    'any(.[]; .action == "step_approved" and .step_id == "s3" and .actor.person_id == $p)' >/dev/null || fail "brak step_approved w audycie"
ok "s3 succeeded, zniknął z kolejki, audyt zawiera step_approved ($PERSON)"

STAGE="settlements-escalation"
esc() { api GET "$API_URL/alerts/$S_ALERT/deliveries" | jq -ce '[.[] | select(.person_id == "kasia" and .status == "delivered")] | select(length > 0)'; }
wait_for "dostarczenie eskalacji do kasia (counterparty-support) w deliveries" esc >/dev/null
ok "eskalacja dostarczona do kasia: $(esc | jq -r '[.[].channel] | join(",")')"

# ---------------------------------------------------------------- (b) payments MANUAL
STAGE="payments-ingest"
REGION="r$RUN"
post_ingest /ingest "$(jq -c --arg id "$REGION" '.payload.labels.region = $id' "$FIX/alertmanager_high_error_rate.json")"
STAGE="payments-alert"
P_ALERT="$(wait_for "alert z $REGION" bash -c "$(declare -f api find_alert); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; find_alert '$REGION'")"
runbooks_of "$P_ALERT" | jq -e 'any(.[]; .runbook_id == "payments-high-error-rate" and .kind == "manual")' >/dev/null \
    || fail "alert $P_ALERT nie ma runbooka payments-high-error-rate"
ok "alert $P_ALERT, runbook MANUAL dołączony (GET /alerts/{id}/runbooks)"

STAGE="payments-delivery"
# preferencja Anny (people.yaml): critical 08-20 -> sms, 20-08 -> email (strefa Europe/Warsaw)
HOUR="$(TZ=Europe/Warsaw date +%-H)"
if [ "$HOUR" -ge 8 ] && [ "$HOUR" -lt 20 ]; then
    sms() { api GET "$API_URL/mock/outbox" | jq -ce --arg a "$P_ALERT" '[.[] | select(.alert_id == $a and .channel == "sms" and .person_id == "anna")] | first // empty'; }
    MSG="$(wait_for "SMS do anna w mock outbox" sms)"
    jq -e --arg l "/alerts/$P_ALERT" '.body | contains($l)' <<<"$MSG" >/dev/null || fail "SMS bez linku do alertu: $MSG"
    jq -e '.body | contains("Open the payments dashboard") | not' <<<"$MSG" >/dev/null || fail "SMS zawiera pełną treść runbooka"
    ok "SMS do anna z linkiem do alertu (runbook MANUAL pod linkiem)"
else
    mail() { local id; id="$(curl -sS -f "$MAILPIT_URL/api/v1/search?query=$REGION" | jq -r '.messages[0].ID // empty')"
        [ -n "$id" ] && curl -sS -f "$MAILPIT_URL/api/v1/message/$id" | jq -ce '.'; }
    MSG="$(wait_for "email z $REGION w Mailpit" mail)"
    jq -e '.Text | contains("Open the payments dashboard")' <<<"$MSG" >/dev/null || fail "email bez treści runbooka MANUAL"
    ok "email w Mailpit zawiera treść runbooka MANUAL"
fi

# ---------------------------------------------------------------- (c) pricing MANUAL, UI only
STAGE="pricing-ingest"
INSTR="EURUSD-$RUN"
post_ingest /ingest/pricing.log.v1 "$(jq -nc --arg i "$INSTR" '{service: "pricing-engine-3", instrument: $i, level: "WARN",
    latency_ms: 1400, message: ("Quote latency 1400 ms for " + $i)}')"
STAGE="pricing-alert"
X_ALERT="$(wait_for "alert pricing $INSTR" bash -c "$(declare -f api find_alert); API_URL='$API_URL' TOKEN='$TOKEN' PERSON='$PERSON'; find_alert '$INSTR'")"
runbooks_of "$X_ALERT" | jq -e 'any(.[]; .runbook_id == "pricing-latency" and .kind == "manual")' >/dev/null \
    || fail "alert $X_ALERT nie ma runbooka pricing-latency (matches po polu service?)"
ok "alert $X_ALERT, runbook MANUAL pricing-latency dołączony"
STAGE="pricing-delivery"
ui() { api GET "$API_URL/alerts/$X_ALERT/deliveries" | jq -ce 'select(length > 0 and all(.[]; .channel == "ui")) | select(any(.[]; .person_id == "tomek")) | select(any(.[]; .status == "delivered"))'; }
wait_for "dostarczenie wyłącznie na ui dla zespołu pricing (w tym tomek)" ui >/dev/null
api GET "$API_URL/mock/outbox" | jq -e --arg a "$X_ALERT" 'all(.[]; .alert_id != $a)' >/dev/null || fail "pricing wysłał SMS/telefon (powinno być UI only)"
ok "pricing: tylko ui (zespół, w tym tomek), brak SMS/telefonu"

STAGE="done"
echo "PASS runbooks: settlements=$S_ALERT payments=$P_ALERT pricing=$X_ALERT"
