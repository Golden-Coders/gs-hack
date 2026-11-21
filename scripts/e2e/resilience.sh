#!/usr/bin/env bash
# 2.1 [19.F1 -> 18.4] — test odporności end-to-end:
#   1) Ruch z loadgen (MODE=both, DUP_RATIO=0.0 dla jednoznacznej liczby unikalnych fingerprintów)
#      w trakcie którego:
#      - zatrzymujemy Mailpit (`docker compose stop mailpit`),
#      - wyłączamy SMS (`PUT /mock/channels/sms`),
#      - restartujemy kolejno: ingest, lifecycle, router, sender,
#      - przywracamy Mailpit i SMS.
#   2) Weryfikacja spójności po ustaniu ruchu:
#      - brak utraconych alertów (każde zdarzenie przyjęte przez ingest tworzy alert),
#      - 0 duplikatów `idempotency_key` w tabeli `deliveries`.
#   3) Weryfikacja fallbacku w API: wyłączony SMS -> status `fallback` na SMS + `delivered` na email.
#   4) Weryfikacja DLQ w API: wyłączone wszystkie kanały -> wpis w `GET /deliveries?status=dlq`
#      -> przywrócenie kanałów -> `POST /deliveries/{id}/resend` -> `delivered`.
#
# Użycie:
#   API_TOKEN=dev-token scripts/e2e/resilience.sh
#   RATE=40 DURATION_SECS=25 API_TOKEN=dev-token scripts/e2e/resilience.sh
set -euo pipefail

source "$(dirname "$0")/lib.sh"
TIMEOUT="${TIMEOUT_SECS:-90}"
RATE="${RATE:-40}"
DURATION_SECS="${DURATION_SECS:-25}"
FIXTURE="$FIXTURES/workflow_failed.json"

set_channel() {
    local ch="$1" avail="$2"
    curl -sS -f -X PUT "$API_URL/mock/channels/$ch" \
        -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" \
        -H 'Content-Type: application/json' \
        --data-binary "{\"available\":$avail}" >/dev/null
}

cleanup() {
    (cd "$ROOT" && docker compose start mailpit >/dev/null 2>&1) || true
    for ch in sms phone email ui; do
        set_channel "$ch" true >/dev/null 2>&1 || true
    done
}
trap cleanup EXIT

wait_health

# --- 0. Stan bazowy -----------------------------------------------------------
STAGE="baseline"
for ch in sms phone email ui; do
    set_channel "$ch" true
done
ALERTS_BEFORE="$(pg_query "SELECT count(*) FROM alerts")"
COUNT_BEFORE="$(pg_query "SELECT coalesce(sum(count), 0) FROM alerts")"
ok "stan bazowy: alerts=$ALERTS_BEFORE sum(count)=$COUNT_BEFORE"

# --- 1. Ruch z loadgen + awarie w trakcie -------------------------------------
STAGE="chaos-load"
LOADGEN_LOG="$(mktemp)"
(
    cd "$ROOT"
    docker compose --profile tools run --rm \
        -e INGEST_URL=http://ingest:8081 \
        -e RATE="$RATE" \
        -e DURATION_SECS="$DURATION_SECS" \
        -e DUP_RATIO=0.0 \
        -e MODE=both \
        loadgen >"$LOADGEN_LOG" 2>&1
) &
LOADGEN_PID=$!

sleep 3
STAGE="chaos-mailpit-sms"
(cd "$ROOT" && docker compose stop mailpit >/dev/null)
set_channel sms false
ok "zatrzymano mailpit i wyłączono SMS w trakcie ruchu"

for svc in ingest lifecycle router sender; do
    sleep 3
    STAGE="restart-$svc"
    (cd "$ROOT" && docker compose restart "$svc" >/dev/null)
    ok "zrestartowano $svc"
done

sleep 3
STAGE="recover"
(cd "$ROOT" && docker compose start mailpit >/dev/null)
set_channel sms true
ok "przywrócono mailpit i SMS"

wait "$LOADGEN_PID" || true
cat "$LOADGEN_LOG"

SENT="$(grep -E '^wysłane:' "$LOADGEN_LOG" | awk '{print $2}' || echo 0)"
ERRORS="$(grep -E '^błędy:' "$LOADGEN_LOG" | awk '{print $2}' || echo 0)"
UNIQUE="$(grep -E '^wygenerowane:' "$LOADGEN_LOG" | sed -E 's/.*unikalne fingerprinty: ([0-9]+).*/\1/' || echo 0)"
rm -f "$LOADGEN_LOG"
[ "${SENT:-0}" -gt 0 ] || fail "loadgen nie wysłał żadnych zdarzeń (SENT=$SENT ERRORS=$ERRORS)"
ok "loadgen zakończony: wysłane=$SENT błędy=$ERRORS (z restartu ingest) unikalne_wygenerowane=$UNIQUE"

# Czekamy aż lifecycle-dedup przetworzy wszystkie przyjęte zdarzenia (zrzut liczników co ~1 s).
STAGE="drain"
EXPECTED_ALERTS=$((ALERTS_BEFORE + SENT))
wait_for "zapisanie co najmniej $SENT nowych alertów (oczekiwano >= $EXPECTED_ALERTS)" bash -c '
    cur="$(docker exec -i "'"$PG_CONTAINER"'" psql -U alerting -tAqc "SELECT count(*) FROM alerts")"
    [ "$cur" -ge "'"$EXPECTED_ALERTS"'" ] && echo "$cur"
' >/dev/null || exit 1

ALERTS_AFTER="$(pg_query "SELECT count(*) FROM alerts")"
COUNT_AFTER="$(pg_query "SELECT coalesce(sum(count), 0) FROM alerts")"
DELTA_ALERTS=$((ALERTS_AFTER - ALERTS_BEFORE))
DELTA_COUNT=$((COUNT_AFTER - COUNT_BEFORE))
ok "po chaosie: delta(alerts)=$DELTA_ALERTS delta(sum_count)=$DELTA_COUNT (przyjęte przez ingest=$SENT)"
[ "$DELTA_ALERTS" -eq "$SENT" ] || fail "delta(alerts)=$DELTA_ALERTS != wysłane=$SENT (przy DUP_RATIO=0.0 każda przyjęta wiadomość daje dokładnie 1 alert)"

DUP_KEYS="$(pg_query "SELECT count(*) FROM (SELECT idempotency_key FROM deliveries GROUP BY idempotency_key HAVING count(*) > 1) d")"
[ "$DUP_KEYS" = "0" ] || fail "wykryto $DUP_KEYS duplikatów idempotency_key w deliveries"
ok "0 duplikatów idempotency_key w deliveries"

# --- 2. Fallback widoczny w API (SMS wyłączony -> fallback na email) ----------
STAGE="fallback"
set_channel sms false
FB_ID="E2E-FB-$(date +%s)-$RANDOM"
ENVELOPE="$(jq -c --arg id "$FB_ID" \
    '.payload.error.code = $id | .payload.workflowId = $id | .payload.timestamp = (now | todate)' "$FIXTURE")"
ingest "$ENVELOPE"
FB_ALERT="$(wait_for "alert fallback $FB_ID" find_alert_by_fp "$FB_ID")" || exit 1
FB_ALERT_ID="$(jq -r .id <<<"$FB_ALERT")"

wait_fallback() {
    api GET "$API_URL/alerts/$FB_ALERT_ID/deliveries" | jq -ce '
        (any(.[]; .channel == "sms" and .status == "fallback"))
        and (any(.[]; .channel == "email" and .status == "delivered"))
        | select(.)'
}
wait_for "fallback sms->email dla alertu $FB_ALERT_ID" wait_fallback >/dev/null || exit 1
set_channel sms true
ok "fallback SMS -> email widoczny w GET /alerts/$FB_ALERT_ID/deliveries"

# --- 3. DLQ widoczne w API + ręczne resend ------------------------------------
STAGE="dlq"
for ch in sms phone email ui; do
    set_channel "$ch" false
done
DLQ_ID="E2E-DLQ-$(date +%s)-$RANDOM"
ENVELOPE="$(jq -c --arg id "$DLQ_ID" \
    '.payload.error.code = $id | .payload.workflowId = $id | .payload.timestamp = (now | todate)' "$FIXTURE")"
ingest "$ENVELOPE"
DLQ_ALERT="$(wait_for "alert dlq $DLQ_ID" find_alert_by_fp "$DLQ_ID")" || exit 1
DLQ_ALERT_ID="$(jq -r .id <<<"$DLQ_ALERT")"

find_dlq_id() {
    api GET "$API_URL/deliveries?status=dlq" | jq -r --arg a "$DLQ_ALERT_ID" \
        '[.[] | select(.alert_id == $a and .status == "dlq")] | first | .id // empty'
}
DLQ_DELIVERY_ID="$(wait_for "wpis DLQ w GET /deliveries?status=dlq dla $DLQ_ALERT_ID" find_dlq_id)" || exit 1
ok "wpis DLQ widoczny w API: delivery_id=$DLQ_DELIVERY_ID"

for ch in sms phone email ui; do
    set_channel "$ch" true
done
api POST "$API_URL/deliveries/$DLQ_DELIVERY_ID/resend" >/dev/null
wait_resent() {
    api GET "$API_URL/alerts/$DLQ_ALERT_ID/deliveries" | jq -ce --arg id "$DLQ_DELIVERY_ID" \
        'any(.[]; .id == $id and .status == "delivered") | select(.)'
}
wait_for "delivered po resend $DLQ_DELIVERY_ID" wait_resent >/dev/null || exit 1
ok "POST /deliveries/$DLQ_DELIVERY_ID/resend przywrócił dostarczenie -> delivered"

DUP_KEYS_END="$(pg_query "SELECT count(*) FROM (SELECT idempotency_key FROM deliveries GROUP BY idempotency_key HAVING count(*) > 1) d")"
[ "$DUP_KEYS_END" = "0" ] || fail "duplikaty idempotency_key po zakończeniu: $DUP_KEYS_END"

STAGE="done"
echo "PASS resilience: alerts_delta=$DELTA_ALERTS dup_keys=0 fallback_alert=$FB_ALERT_ID dlq_delivery=$DLQ_DELIVERY_ID"
