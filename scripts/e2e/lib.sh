# Wspólne helpery skryptów e2e: source "$(dirname "$0")/lib.sh"
# Zmienne: API_URL, INGEST_URL (domyślnie :8081 — /ingest serwuje binarka ingest, nie api),
#          API_TOKEN, PERSON_ID, TIMEOUT_SECS (limit czasu każdego etapu),
#          PG_CONTAINER (kontener Postgresa do odczytu kolumn spoza API, domyślnie gs-hack-postgres-1).
# Zależności: curl, jq; docker tylko do odczytu z Postgresa (pg_query) i przy COMPOSE_UP=1.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
API_URL="${API_URL:-http://localhost:${API_PORT:-8080}}"
INGEST_URL="${INGEST_URL:-http://localhost:8081}"
TOKEN="${API_TOKEN:-dev-token}"
PERSON="${PERSON_ID:-e2e}"
TIMEOUT="${TIMEOUT_SECS:-60}"
PG_CONTAINER="${PG_CONTAINER:-gs-hack-postgres-1}"
FIXTURES="$ROOT/crates/contracts/fixtures/events"

STAGE="start"
fail() { echo "FAIL [$STAGE]: $*" >&2; exit 1; }
ok() { echo "OK   [$STAGE] $*"; }
skip() { echo "SKIP [$STAGE] $*"; }

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

# wait_for_quiet <polecenie...>: jak wait_for, ale bez fail (zwraca 1 po limicie).
wait_for_quiet() {
    local deadline=$((SECONDS + TIMEOUT)) out
    while :; do
        if out="$("$@" 2>/dev/null)" && [ -n "$out" ]; then printf '%s' "$out"; return 0; fi
        [ "$SECONDS" -ge "$deadline" ] && return 1
        sleep 1
    done
}

# outbox_covers <iso-ts>: 0, gdy okno GET /mock/outbox (ostatnie 200 wpisów) sięga wstecz przed <iso-ts>.
# Pod obciążeniem wpisy wypadają z okna — wtedy asercje na outboxie trzeba pominąć (SKIP),
# a źródłem prawdy zostaje GET /alerts/{id}/deliveries.
outbox_covers() {
    api GET "$API_URL/mock/outbox" | jq -e --arg t "$1" '
        def sec: sub("\\.[0-9]+"; "") | fromdate;
        length < 200 or ((map(.sent_at) | min | sec) < ($t | sec))' >/dev/null
}

# compose_up: przy COMPOSE_UP=1 najpierw docker compose up -d --build.
compose_up() {
    if [ "${COMPOSE_UP:-0}" = "1" ]; then
        STAGE="compose"
        command -v docker >/dev/null || fail "brak programu docker"
        (cd "$ROOT" && docker compose up -d --build) || fail "docker compose up nie powiódł się"
    fi
}

# wait_health: czeka na api (/health jest publiczne), potem sprawdza token na chronionym endpoincie;
# 401/403 to zły API_TOKEN — fail od razu zamiast ponawiać aż do TIMEOUT.
wait_health() {
    STAGE="health"
    wait_for "odpowiedź $API_URL/health" api GET "$API_URL/health" >/dev/null || exit 1
    local code
    code="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $TOKEN" \
        -H "X-Person-Id: $PERSON" "$API_URL/alerts?limit=1" 2>/dev/null)"
    if [ "$code" = "401" ] || [ "$code" = "403" ]; then
        STAGE="auth"; fail "api zwraca $code — sprawdź API_TOKEN"
    fi
    ok "api $API_URL odpowiada"
}

# ingest <envelope-json>: POST /ingest, oczekuje 202.
ingest() {
    local code
    code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$INGEST_URL/ingest" \
        -H "Authorization: Bearer $TOKEN" -H "X-Person-Id: $PERSON" \
        -H 'Content-Type: application/json' --data-binary "$1")" || fail "ingest $INGEST_URL niedostępny"
    [ "$code" = "202" ] || fail "POST $INGEST_URL/ingest zwrócił $code (oczekiwano 202)"
}

# find_alert_by_fp <podciąg>: alert, którego fingerprint zawiera podciąg.
# GET /alerts nie ma filtra po fingerprincie, a pod obciążeniem alert wypada z okna limit=1000,
# więc fallback: id z Postgresa (alerts.fingerprint, tylko odczyt), potem GET /alerts/{id}
# (nadzbiór AlertSummary).
find_alert_by_fp() {
    local found id
    found="$(api GET "$API_URL/alerts?limit=1000" | jq -ce --arg id "$1" \
        '[.[] | select(.fingerprint | contains($id))] | first // empty')"
    if [ -n "$found" ]; then printf '%s' "$found"; return 0; fi
    command -v docker >/dev/null || return 1
    id="$(pg_query "SELECT id FROM alerts WHERE strpos(fingerprint, \$fp\$$1\$fp\$) > 0 ORDER BY updated_at DESC LIMIT 1")" || return 1
    [ -n "$id" ] || return 1
    api GET "$API_URL/alerts/$id" | jq -ce 'select(.id != null)'
}

# pg_query <sql>: zapytanie tylko do odczytu (np. deliveries.request, którego nie ma w API).
pg_query() {
    docker exec -i "$PG_CONTAINER" psql -U alerting -tAqc "$1"
}
