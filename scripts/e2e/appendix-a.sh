#!/usr/bin/env bash
# 19.D1 / 18.2 — trzy zdarzenia z Appendix A (docs/exercise.md) => alerty i dostarczenia zgodne
# z regułami z config/rules/ i preferencjami z config/people.yaml.
# 19.D2 — dedup i reopen end-to-end (krok 2 tego skryptu).
#
# Oczekiwania (severity alertu z mappingu config/mappings/, severity dostarczenia = severity_override reguły):
#
# | Zdarzenie (schema)                       | sev. mappingu | Reguła                 | sev. dostarczenia | Odbiorcy (skąd)                     | Kanał główny (skąd)                                   | Łańcuch (request.chain)        |
# |------------------------------------------|---------------|------------------------|-------------------|-------------------------------------|-------------------------------------------------------|--------------------------------|
# | WORKFLOW_FAILED order-settlement prod    | error         | settlements-prod       | critical          | jan, ola (zespół settlements)       | phone + sms (domyślne zespołu settlements, critical)   | phone→email→ui, sms→email→ui   |
# |   (workflow.v1)                          |               |                        |                   |                                     |                                                       |                                |
# | HighErrorRate payments-api (alertmanager)| critical      | anna-payments-critical | critical          | anna (osoba)                        | własne preferencje Anny: sms 08–20, email 20–08 (W-wa) | sms→email→ui / email→ui        |
# | trade rejected rates 25M EUR             | critical      | rates-big-rejections   | critical          | piotr, anna (zespół rates)          | piotr: email (własne); anna: sms 08–20 / email 20–08   | email→ui; anna jak wyżej       |
# |   (trade.rejected.v2)                    |               |                        |                   |                                     |                                                       |                                |
#
# Reguła pricing-warnings nie pasuje do żadnego zdarzenia (brak pricing-engine-*), więc tomek nic
# nie dostaje; asercja na dokładny zbiór (osoba, kanał) to sprawdza. W GET /mock/outbox lądują
# tylko sms i phone (email idzie do Mailpit, ui do feedu), więc oczekiwany outbox to podzbiór.
# Źródłem prawdy są GET /alerts/{id}/deliveries; GET /mock/outbox zwraca tylko 200 ostatnich
# wpisów, więc gdy pod obciążeniem okno nie obejmuje już alertu, asercje outboxu dają SKIP.
#
# Anna (godziny): router liczy kanał z czasu zdarzenia w strefie osoby (Europe/Warsaw). Skrypt liczy
# oczekiwanie z bieżącej godziny w tej strefie; w oknie ±2 min wokół 08:00/20:00 wynik nie jest
# deterministyczny — wtedy SKIP dla kanału Anny (sprawdzane jest tylko, że dostała dokładnie jedno
# z sms/email). Scenariusz „Anna o 22:00” z dowolną godziną testuje jednostkowo router.
#
# Fallback na UI: każdy łańcuch w deliveries.request (kolumna niewystawiona w API, odczyt przez
# psql w kontenerze Postgresa, tylko SELECT) kończy się na `ui`.
#
# Dedup/reopen (D2): 500 duplikatów WORKFLOW_FAILED z unikalnym error.code => 1 alert z count 500,
# dokładnie jedno dostarczenie na (jan|ola, sms|phone) i po jednym sms/phone w outboxie; potem
# POST /alerts/{id}/resolve, nowe zdarzenie z tym samym fingerprintem => ten sam alert_id,
# epizod 2 w GET /alerts/{id}/episodes i ponowne dostarczenia w epizodzie 2.
#
# Uwaga: GET /alerts zwraca severity z mappingu, a nie severity_override reguły, oraz puste
# teams/rule_ids (otwarta kwestia D10), więc severity sprawdzamy na dostarczeniach (request.severity).
#
# Działa na niepustym systemie (unikalne identyfikatory per przebieg).
# Użycie: API_URL=http://localhost:8088 scripts/e2e/appendix-a.sh
#         SKIP_DEDUP=1 ...  # tylko D1;  DUPLICATES=500 (liczba duplikatów w D2)
# Zmienne wspólne: scripts/e2e/lib.sh. Zależności: curl, jq, docker (odczyt request z Postgresa).
set -euo pipefail

source "$(dirname "$0")/lib.sh"
DUPLICATES="${DUPLICATES:-500}"

compose_up
wait_health

STAGE="channels"
CH="$(api GET "$API_URL/mock/channels")" || fail "GET /mock/channels"
jq -e 'all(.available)' <<<"$CH" >/dev/null \
    || fail "nie wszystkie symulowane kanały są dostępne: $(jq -c . <<<"$CH") (PUT /mock/channels/{channel})"
ok "kanały sms/phone/email dostępne"

STAGE="anna-hours"
HM=$((10#$(TZ=Europe/Warsaw date +%H%M)))
if { [ "$HM" -ge 758 ] && [ "$HM" -le 802 ]; } || { [ "$HM" -ge 1958 ] && [ "$HM" -le 2002 ]; }; then
    ANNA_CH="?"
    skip "Europe/Warsaw $(TZ=Europe/Warsaw date +%H:%M) — blisko granicy 08:00/20:00, kanał Anny niedeterministyczny"
elif [ "$HM" -ge 800 ] && [ "$HM" -lt 2000 ]; then
    ANNA_CH="sms"; ok "Europe/Warsaw $(TZ=Europe/Warsaw date +%H:%M) => Anna: sms"
else
    ANNA_CH="email"; ok "Europe/Warsaw $(TZ=Europe/Warsaw date +%H:%M) => Anna: email"
fi

# exp <person> <channel>: oczekiwany wpis {p, c, chain} wg default_fallback_chain (router).
exp() {
    local p="$1" c="$2"
    if [ "$p" = "anna" ] && [ "$ANNA_CH" = "?" ]; then jq -nc '{p:"anna", c:"?", chain:null}'; return; fi
    local fb
    fb="$(jq -nc --arg p "$p" '{anna:["email"],jan:["email"],ola:["email"],piotr:["ui"]}[$p]')"
    jq -nc --arg p "$p" --arg c "$c" --argjson fb "$fb" '
        {p:$p, c:$c, chain: (if $c == "ui" then ["ui"]
                             else ([$c] + ($fb - ["ui", $c])) + ["ui"] end)}'
}

# pg_deliveries <alert_id> [episode]: dostarczenia z request (severity, chain, origin).
pg_deliveries() {
    local ep="${2:-}"
    pg_query "SELECT coalesce(json_agg(json_build_object('p',person_id,'c',channel,'episode',episode,
        'status',status,'sev',request->'severity','chain',request->'chain','rule',request->'origin'->>'rule_id')
        ORDER BY person_id, channel), '[]') FROM deliveries WHERE alert_id = '$1'
        ${ep:+AND episode = $ep}"
}

# wait_delivered <alert_id> <episode> <expected-json>: czeka, aż zbiór (p, c) z GET /alerts/{id}/deliveries
# w epizodzie ze statusem delivered będzie równy oczekiwanemu (anna z c="?" = dowolny z sms/email).
NORM='map(if .p == "anna" and $anna == "?" then .c = "?" else . end) | map({p, c}) | sort'
wait_delivered() {
    DW_ID="$1"; DW_EP="$2"; DW_WANT="$3"
    local out actual
    if ! out="$(wait_for_quiet delivered_match)"; then
        actual="$(api GET "$API_URL/alerts/$DW_ID/deliveries" | jq -c --argjson ep "$DW_EP" \
            '[.[] | select(.episode == $ep) | {p: .person_id, c: .channel, status}]')"
        fail "alert $DW_ID ep $DW_EP: oczekiwano delivered $(jq -c --arg anna "$ANNA_CH" "$NORM" <<<"$DW_WANT"), jest $actual"
    fi
    printf '%s' "$out"
}
delivered_match() {
    api GET "$API_URL/alerts/$DW_ID/deliveries" | jq -ce --argjson ep "$DW_EP" --argjson want "$DW_WANT" --arg anna "$ANNA_CH" "
        [.[] | select(.episode == \$ep and .status == \"delivered\")] as \$d
        | if ([\$d[] | {p: .person_id, c: .channel}] | $NORM) == (\$want | $NORM) then \$d else empty end"
}

# outbox_for <alert_id>: posortowane [{p, c}] z GET /mock/outbox dla alertu.
outbox_for() {
    api GET "$API_URL/mock/outbox" | jq -c --arg a "$1" '[.[] | select(.alert_id == $a) | {p: .person_id, c: .channel}] | sort'
}

# check_flow <etykieta> <podciąg fingerprintu> <schema> <reguła> <expected-json>
check_flow() {
    local label="$1" fp="$2" schema="$3" rule="$4" want="$5" alert id d req want_ob ob
    STAGE="$label/alert"
    alert="$(wait_for "alert z fingerprintem zawierającym $fp w GET /alerts" find_alert_by_fp "$fp")" || exit 1
    id="$(jq -r .id <<<"$alert")"
    [ "$(jq -r .schema <<<"$alert")" = "$schema" ] || fail "alert $id ma schema $(jq -r .schema <<<"$alert"), oczekiwano $schema"
    [ "$(jq -r .state <<<"$alert")" = "open" ] || fail "alert $id nie jest open"
    ok "alert $id schema=$schema severity(mapping)=$(jq -r .severity <<<"$alert") count=$(jq -r .count <<<"$alert")"

    STAGE="$label/deliveries"
    d="$(wait_delivered "$id" 1 "$want")" || exit 1
    jq -r '.[] | "       \(.person_id) \(.channel) \(.status) delivered_at=\(.delivered_at)"' <<<"$d"
    ok "dostarczenia (osoba, kanał) zgodne z oczekiwaniem"

    STAGE="$label/request"
    if ! req="$(pg_deliveries "$id" 1)"; then
        skip "brak odczytu deliveries.request z $PG_CONTAINER — pomijam severity/łańcuch/regułę"
    else
        jq -e --arg sev critical --arg rule "$rule" 'all(.sev == $sev and .rule == $rule and .chain[-1] == "ui" and .chain[0] == .c)' <<<"$req" >/dev/null \
            || fail "request: oczekiwano severity=critical, rule=$rule, chain[0]=kanał, chain[-1]=ui; jest $(jq -c 'map({p, c, sev, rule, chain})' <<<"$req")"
        jq -e --argjson want "$want" '
            [.[] | {p, c, chain}] as $got
            | all($want[] | select(.chain != null); . as $w | $got | index($w) != null)' <<<"$req" >/dev/null \
            || fail "łańcuchy fallbacku: oczekiwano $(jq -c '[.[] | select(.chain != null)]' <<<"$want"), jest $(jq -c 'map({p, c, chain})' <<<"$req")"
        jq -r '.[] | "       \(.p) \(.c) severity=\(.sev) rule=\(.rule) chain=\(.chain | join("→"))"' <<<"$req"
        ok "request.severity=critical, reguła $rule, łańcuchy kończą się na ui"
    fi

    STAGE="$label/outbox"
    # Anna przy niedeterministycznym kanale ("?") jest pomijana w porównaniu outboxu.
    want_ob="$(jq -c '[.[] | select(.c == "sms" or .c == "phone") | {p, c}] | sort' <<<"$want")"
    OB_ID="$id"; OB_WANT="$want_ob"
    ob_match() {
        local got
        got="$(outbox_for "$OB_ID" | jq -c --arg anna "$ANNA_CH" '[.[] | select(.p != "anna" or $anna != "?")]')"
        [ "$got" = "$OB_WANT" ] && echo "$got"
    }
    if ! outbox_covers "$(jq -r 'map(.received_at) | min' <<<"$d")"; then
        skip "okno GET /mock/outbox (200 wpisów) nie obejmuje już alertu $id — źródłem prawdy są deliveries"
    elif ob="$(wait_for_quiet ob_match)"; then
        ok "outbox (sms/phone): $ob"
    else
        fail "outbox dla $id: oczekiwano $want_ob, jest $(outbox_for "$id")"
    fi
    LAST_ALERT_ID="$id"
}

# --- 1. trzy zdarzenia z Appendix A (D1) ----------------------------------------
STAGE="ingest"
RUN_ID="E2E-A-$(date +%s)-$RANDOM"
ingest "$(jq -c --arg id "$RUN_ID" '.payload.error.code = ($id + "-wf") | .payload.workflowId = ("WF-" + $id)
    | .payload.timestamp = (now | todate)' "$FIXTURES/workflow_failed.json")"
ingest "$(jq -c --arg id "$RUN_ID" '.payload.alertname = ("HighErrorRate-" + $id + "-am")
    | .payload.startsAt = (now | todate)' "$FIXTURES/alertmanager_high_error_rate.json")"
ingest "$(jq -c --arg id "$RUN_ID" '.payload.payload.tradeId = ("T-" + $id + "-tr")
    | .payload.meta.ts = (now * 1000 | floor)' "$FIXTURES/trade_rejected.json")"
ok "3 zdarzenia przyjęte (RUN_ID=$RUN_ID)"

check_flow "workflow" "$RUN_ID-wf" "workflow.v1" "settlements-prod" \
    "[$(exp jan phone),$(exp jan sms),$(exp ola phone),$(exp ola sms)]"
check_flow "alertmanager" "$RUN_ID-am" "alertmanager" "anna-payments-critical" \
    "[$(exp anna "$ANNA_CH")]"
check_flow "trade" "$RUN_ID-tr" "trade.rejected.v2" "rates-big-rejections" \
    "[$(exp piotr email),$(exp anna "$ANNA_CH")]"

STAGE="appendix-a"
echo "PASS appendix-a (D1): 3 zdarzenia, 3 reguły, dostarczenia wg preferencji, łańcuchy kończą się na ui"

if [ "${SKIP_DEDUP:-0}" = "1" ]; then exit 0; fi

# --- 2. dedup + reopen (D2) ----------------------------------------------------
STAGE="dedup/ingest"
DUP_ID="E2E-DUP-$(date +%s)-$RANDOM"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
jq -c --arg id "$DUP_ID" '.payload.error.code = $id | .payload.workflowId = ("WF-" + $id)' \
    "$FIXTURES/workflow_failed.json" > "$TMP/event.json"
for i in $(seq "$DUPLICATES"); do
    [ "$i" -gt 1 ] && echo next
    printf 'url = "%s/ingest"\nrequest = "POST"\nheader = "Authorization: Bearer %s"\nheader = "X-Person-Id: %s"\nheader = "Content-Type: application/json"\ndata-binary = "@%s"\noutput = "/dev/null"\nwrite-out = "%%{http_code}\\n"\n' \
        "$INGEST_URL" "$TOKEN" "$PERSON" "$TMP/event.json"
done > "$TMP/curl.cfg"
codes="$(curl -sS -Z --parallel-max 16 -K "$TMP/curl.cfg")" || fail "ingest $INGEST_URL niedostępny"
accepted="$(grep -c '^202$' <<<"$codes" || true)"
[ "$accepted" = "$DUPLICATES" ] || fail "przyjęto $accepted/$DUPLICATES (kody: $(sort <<<"$codes" | uniq -c | tr '\n' ' '))"
ok "$DUPLICATES duplikatów przyjętych (error.code=$DUP_ID)"

STAGE="dedup/alert"
count_reached() {
    api GET "$API_URL/alerts?limit=1000" | jq -ce --arg id "$DUP_ID" --argjson n "$DUPLICATES" \
        '[.[] | select(.fingerprint | contains($id))] as $a | if ($a | length) == 1 and $a[0].count >= $n then $a[0] else empty end'
}
if ! ALERT="$(wait_for_quiet count_reached)"; then
    fail "oczekiwano 1 alertu z count=$DUPLICATES, jest $(api GET "$API_URL/alerts?limit=1000" | jq -c --arg id "$DUP_ID" '[.[] | select(.fingerprint | contains($id)) | {id, count, state}]')"
fi
DUP_ALERT="$(jq -r .id <<<"$ALERT")"
sleep 2 # zrzut licznika co ~1 s — upewnij się, że nic nie dojdzie
ALERT="$(api GET "$API_URL/alerts/$DUP_ALERT")"
[ "$(jq -r .count <<<"$ALERT")" = "$DUPLICATES" ] || fail "count=$(jq -r .count <<<"$ALERT"), oczekiwano $DUPLICATES"
n_alerts="$(api GET "$API_URL/alerts?limit=1000" | jq --arg id "$DUP_ID" '[.[] | select(.fingerprint | contains($id))] | length')"
[ "$n_alerts" = "1" ] || fail "alertów z fingerprintem $DUP_ID: $n_alerts, oczekiwano 1"
ok "1 alert $DUP_ALERT count=$DUPLICATES episode=$(jq -r .episode <<<"$ALERT")"

DUP_WANT="[$(exp jan phone),$(exp jan sms),$(exp ola phone),$(exp ola sms)]"
STAGE="dedup/deliveries"
wait_delivered "$DUP_ALERT" 1 "$DUP_WANT" >/dev/null || exit 1
sleep 2
rows="$(api GET "$API_URL/alerts/$DUP_ALERT/deliveries" | jq -c '[.[] | {p: .person_id, c: .channel, episode}] | sort')"
[ "$(jq length <<<"$rows")" = "4" ] || fail "oczekiwano dokładnie 4 dostarczeń (jan|ola × sms|phone), jest $rows"
DUP_SINCE="$(api GET "$API_URL/alerts/$DUP_ALERT/deliveries" | jq -r 'map(.received_at) | min')"
if outbox_covers "$DUP_SINCE"; then
    ob="$(outbox_for "$DUP_ALERT")"
    [ "$ob" = "$(jq -c '[.[] | {p, c}] | sort' <<<"$DUP_WANT")" ] \
        || fail "outbox: oczekiwano po jednym sms/phone dla jan i ola, jest $ob"
else
    skip "okno GET /mock/outbox nie obejmuje alertu $DUP_ALERT — sprawdzone tylko deliveries"
fi
ok "dokładnie jedno dostarczenie na (odbiorca, kanał): $rows"

STAGE="dedup/resolve"
code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$API_URL/alerts/$DUP_ALERT/resolve" \
    -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON")"
[ "${code:0:1}" = "2" ] || fail "POST /alerts/$DUP_ALERT/resolve zwrócił $code"
resolved() { api GET "$API_URL/alerts/$DUP_ALERT" | jq -ce 'select(.state == "resolved")'; }
wait_for "alert $DUP_ALERT w stanie resolved" resolved >/dev/null || exit 1
ok "alert $DUP_ALERT resolved (HTTP $code)"

STAGE="dedup/reopen"
ingest "$(cat "$TMP/event.json")"
reopened() { api GET "$API_URL/alerts/$DUP_ALERT" | jq -ce 'select(.state == "reopened" and .episode == 2)'; }
ALERT="$(wait_for "alert $DUP_ALERT w stanie reopened w epizodzie 2" reopened)" || exit 1
n_alerts="$(api GET "$API_URL/alerts?limit=1000" | jq --arg id "$DUP_ID" '[.[] | select(.fingerprint | contains($id))] | length')"
[ "$n_alerts" = "1" ] || fail "po reopen alertów z fingerprintem $DUP_ID: $n_alerts, oczekiwano 1 (ten sam alert_id)"
EP="$(api GET "$API_URL/alerts/$DUP_ALERT/episodes")"
jq -e 'length == 2 and (map(.episode) | sort) == [1, 2] and (.[] | select(.episode == 1) | .resolved_at != null)
       and (.[] | select(.episode == 2) | .resolved_at == null)' <<<"$EP" >/dev/null \
    || fail "epizody: oczekiwano [1 (resolved), 2 (open)], jest $(jq -c 'map({episode, count, resolved_at})' <<<"$EP")"
ok "ten sam alert $DUP_ALERT, epizody: $(jq -c 'map({episode, count, resolved_at})' <<<"$EP")"

STAGE="dedup/redelivery"
wait_delivered "$DUP_ALERT" 2 "$DUP_WANT" >/dev/null || exit 1
expect_ob="$(jq -c '[.[] | {p, c}] | (. + .) | sort' <<<"$DUP_WANT")"
ob_two() { o="$(outbox_for "$DUP_ALERT")"; [ "$o" = "$expect_ob" ] && echo "$o"; }
if ! outbox_covers "$DUP_SINCE"; then
    skip "okno GET /mock/outbox nie obejmuje alertu $DUP_ALERT — sprawdzone tylko deliveries"
else wait_for_quiet ob_two >/dev/null || fail "outbox po reopen: oczekiwano po 2× sms/phone dla jan i ola, jest $(outbox_for "$DUP_ALERT")"
fi
ok "ponowne dostarczenia w epizodzie 2: $(api GET "$API_URL/alerts/$DUP_ALERT/deliveries" | jq -c '[.[] | select(.episode == 2) | "\(.person_id)/\(.channel)/\(.status)"]')"

STAGE="done"
echo "PASS appendix-a (D1 + D2): dedup $DUPLICATES => 1 alert $DUP_ALERT, reopen w epizodzie 2 z ponownym powiadomieniem"
