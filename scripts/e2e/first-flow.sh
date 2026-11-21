#!/usr/bin/env bash
# 19.C1 / 18.1 — pierwszy przepływ end-to-end:
#   POST /ingest (WORKFLOW_FAILED) -> alert w GET /alerts -> wpisy w GET /alerts/{id}/deliveries
#   z received_at/delivered_at -> SMS w GET /mock/outbox (gdy okno 200 wpisów obejmuje alert).
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
# Zmienne i helpery: scripts/e2e/lib.sh. Zależności: curl, jq, docker (tylko przy COMPOSE_UP=1).
set -euo pipefail

source "$(dirname "$0")/lib.sh"
FIXTURE="$FIXTURES/workflow_failed.json"
RECIPIENTS='["jan","ola"]'

compose_up
wait_health

# --- 1. ingest ----------------------------------------------------------------
STAGE="ingest"
RUN_ID="E2E-$(date +%s)-$RANDOM"
ENVELOPE="$(jq -c --arg id "$RUN_ID" \
    '.payload.error.code = $id | .payload.workflowId = $id | .payload.timestamp = (now | todate)' "$FIXTURE")"
ingest "$ENVELOPE"
ok "zdarzenie przyjęte (error.code=$RUN_ID)"

# --- 2. alert -----------------------------------------------------------------
STAGE="alert"
ALERT="$(wait_for "alert z fingerprintem zawierającym $RUN_ID w GET /alerts (normalizer/dedup w lifecycle?)" find_alert_by_fp "$RUN_ID")" || exit 1
ALERT_ID="$(jq -r .id <<<"$ALERT")"
ok "alert $ALERT_ID state=$(jq -r .state <<<"$ALERT") severity=$(jq -r .severity <<<"$ALERT") fingerprint=$(jq -r .fingerprint <<<"$ALERT")"
[ "$(jq -r .state <<<"$ALERT")" = "open" ] || fail "alert $ALERT_ID nie jest open"

# --- 3. dostarczenia (źródło prawdy) -----------------------------------------------------------
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

# --- 4. SMS w mock outbox (okno 200 ostatnich wpisów; pod obciążeniem SKIP) ---
STAGE="outbox"
find_sms() {
    api GET "$API_URL/mock/outbox" | jq -ce --arg a "$ALERT_ID" --argjson who "$RECIPIENTS" '
        [.[] | select(.alert_id == $a and .channel == "sms")] as $m
        | if ($who - [$m[].person_id] | length) == 0 then $m else empty end'
}
if ! outbox_covers "$(jq -r 'map(.received_at) | min' <<<"$DELIVERIES")"; then
    skip "okno GET /mock/outbox (200 wpisów) nie obejmuje już alertu $ALERT_ID — źródłem prawdy są deliveries"
else
    SMS="$(wait_for "SMS do $RECIPIENTS dla alertu $ALERT_ID w GET /mock/outbox (router/sender?)" find_sms)" || exit 1
    jq -r '.[] | "       sms -> \(.person_id) \(.to): \(.title)"' <<<"$SMS"
    ok "SMS wysłane do: $RECIPIENTS"
fi

STAGE="done"
echo "PASS first-flow: alert $ALERT_ID"
