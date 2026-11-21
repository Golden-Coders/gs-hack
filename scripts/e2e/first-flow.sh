#!/usr/bin/env bash
# 19.C1 / 18.1 — pierwszy przepływ end-to-end:
#   POST /ingest (WORKFLOW_FAILED) -> alert w GET /alerts -> SMS w GET /mock/outbox
#   -> wpisy w GET /alerts/{id}/deliveries z received_at/delivered_at.
#
# Reguła `settlements-prod` (config/p2/rules/settlements-prod.yaml) kieruje alert (severity
# nadpisane na critical) do zespołu `settlements` (jan, ola). Oboje nie mają własnych
# preferencji, więc dostają kanały domyślne zespołu dla critical: phone + sms
# (config/p2/people.yaml). Skrypt sprawdza SMS dla obojga.
#
# Działa na niepustym systemie: payload dostaje unikalny `error.code` (składnik fingerprintu
# w config/p1/mappings/workflow.v1.yaml), więc za każdym razem powstaje nowy alert.
#
# Użycie:
#   scripts/e2e/first-flow.sh
#   API_URL=http://localhost:8088 scripts/e2e/first-flow.sh   # gdy api jest na innym porcie
#   COMPOSE_UP=1 scripts/e2e/first-flow.sh                     # najpierw docker compose up -d --build
# Zmienne: API_URL, INGEST_URL (domyślnie :8081 — /ingest serwuje binarka ingest, nie api),
#          API_TOKEN, PERSON_ID, TIMEOUT_SECS (limit czasu każdego etapu).
# Zależności: curl, jq, docker (tylko przy COMPOSE_UP=1).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
API_URL="${API_URL:-http://localhost:${API_PORT:-8080}}"
INGEST_URL="${INGEST_URL:-http://localhost:8081}"
TOKEN="${API_TOKEN:-dev-token}"
PERSON="${PERSON_ID:-e2e}"
TIMEOUT="${TIMEOUT_SECS:-60}"
FIXTURE="$ROOT/crates/contracts/fixtures/events/workflow_failed.json"
RECIPIENTS='["jan","ola"]'

STAGE="start"
fail() { echo "FAIL [$STAGE]: $*" >&2; exit 1; }
ok() { echo "OK   [$STAGE] $*"; }

for bin in curl jq; do command -v "$bin" >/dev/null || fail "brak programu $bin"; done

api() { # api <METHOD> <URL>
    curl -sS -f -X "$1" -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" "$2"
}

# wait_for <opis> <polecenie...>: powtarza polecenie co 1 s, aż zwróci 0 i niepuste wyjście.
wait_for() {
    local what="$1"; shift
    local deadline=$((SECONDS + TIMEOUT)) out
    while :; do
        if out="$("$@" 2>/dev/null)" && [ -n "$out" ]; then printf '%s' "$out"; return 0; fi
        if [ "$SECONDS" -ge "$deadline" ]; then fail "po ${TIMEOUT}s brak: $what"; fi
        sleep 1
    done
}

# --- 0. stack -----------------------------------------------------------------
if [ "${COMPOSE_UP:-0}" = "1" ]; then
    STAGE="compose"
    command -v docker >/dev/null || fail "brak programu docker"
    (cd "$ROOT" && docker compose up -d --build) || fail "docker compose up nie powiódł się"
fi

STAGE="health"
wait_for "odpowiedź $API_URL/health" api GET "$API_URL/health" >/dev/null || exit 1
ok "api $API_URL odpowiada"

# --- 1. ingest ----------------------------------------------------------------
STAGE="ingest"
RUN_ID="E2E-$(date +%s)-$RANDOM"
ENVELOPE="$(jq -c --arg id "$RUN_ID" \
    '.payload.error.code = $id | .payload.workflowId = $id | .payload.timestamp = (now | todate)' "$FIXTURE")"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$INGEST_URL/ingest" \
    -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" \
    -H 'Content-Type: application/json' --data-binary "$ENVELOPE")" || fail "ingest $INGEST_URL niedostępny"
[ "$code" = "202" ] || fail "POST $INGEST_URL/ingest zwrócił $code (oczekiwano 202)"
ok "zdarzenie przyjęte (error.code=$RUN_ID)"

# --- 2. alert -----------------------------------------------------------------
STAGE="alert"
find_alert() {
    api GET "$API_URL/alerts?limit=1000" | jq -ce --arg id "$RUN_ID" \
        '[.[] | select(.fingerprint | contains($id))] | first // empty'
}
ALERT="$(wait_for "alert z fingerprintem zawierającym $RUN_ID w GET /alerts (normalizer/dedup w lifecycle?)" find_alert)" || exit 1
ALERT_ID="$(jq -r .id <<<"$ALERT")"
ok "alert $ALERT_ID state=$(jq -r .state <<<"$ALERT") severity=$(jq -r .severity <<<"$ALERT") fingerprint=$(jq -r .fingerprint <<<"$ALERT")"
[ "$(jq -r .state <<<"$ALERT")" = "open" ] || fail "alert $ALERT_ID nie jest open"

# --- 3. SMS w mock outbox ------------------------------------------------------
STAGE="outbox"
find_sms() {
    api GET "$API_URL/mock/outbox" | jq -ce --arg a "$ALERT_ID" --argjson who "$RECIPIENTS" '
        [.[] | select(.alert_id == $a and .channel == "sms")] as $m
        | if ($who - [$m[].person_id] | length) == 0 then $m else empty end'
}
SMS="$(wait_for "SMS do $RECIPIENTS dla alertu $ALERT_ID w GET /mock/outbox (router/sender?)" find_sms)" || exit 1
jq -r '.[] | "       sms -> \(.person_id) \(.to): \(.title)"' <<<"$SMS"
ok "SMS wysłane do: $RECIPIENTS"

# --- 4. dostarczenia -----------------------------------------------------------
STAGE="deliveries"
find_deliveries() {
    api GET "$API_URL/alerts/$ALERT_ID/deliveries" | jq -ce --argjson who "$RECIPIENTS" '
        [.[] | select(.channel == "sms" and .status == "delivered"
                      and (.received_at // "") != "" and (.delivered_at // "") != "")] as $d
        | if ($who - [$d[].person_id] | length) == 0 then . else empty end'
}
DELIVERIES="$(wait_for "dostarczenia sms (delivered, z received_at i delivered_at) dla $RECIPIENTS w GET /alerts/$ALERT_ID/deliveries" find_deliveries)" || exit 1
jq -r '.[] | "       \(.person_id) \(.channel) \(.status) attempts=\(.attempts) received_at=\(.received_at) delivered_at=\(.delivered_at)"' <<<"$DELIVERIES"
ok "dostarczenia SMS zapisane z received_at i delivered_at"

STAGE="done"
echo "PASS first-flow: alert $ALERT_ID"
