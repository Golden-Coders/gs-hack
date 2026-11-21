#!/usr/bin/env bash
# Generator scenariusza demo (idempotentny, nie dotyka stacku). Użycie: scripts/demo/run.sh
# Zmienne: API_URL, INGEST_URL, API_TOKEN, DUPS (domyślnie 500), DEMO_OUTAGE=0 pomija krok (d).
set -euo pipefail
cd "$(dirname "$0")/../.."
API_URL=${API_URL:-http://localhost:8080}
INGEST_URL=${INGEST_URL:-http://localhost:8081}
API_TOKEN=${API_TOKEN:-dev-token}
DEMO_OUTAGE=${DEMO_OUTAGE:-1}
DUPS=${DUPS:-500}
FIX=crates/contracts/fixtures/events
RUN=$(date +%s)
export INGEST_URL FIX

api() { curl -sf -H "Authorization: Bearer $API_TOKEN" -H "X-Person-Id: anna" -H "Content-Type: application/json" "$@"; }
ingest() { curl -sf -o /dev/null -X POST -H "Content-Type: application/json" --data-binary @- "$INGEST_URL/ingest"; }
# patch <plik> <kod python na zmiennej p = payload>
patch() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); p=d["payload"]; exec(sys.argv[2]); print(json.dumps(d))' "$1" "$2"; }
sms() { api -X PUT "$API_URL/mock/channels/sms" -d "{\"available\":$1}" >/dev/null; }
trap '[ "$DEMO_OUTAGE" = 1 ] && sms true || true' EXIT   # SMS zawsze wraca do działania

echo "== (a) trzy schematy z Appendix A"
for f in workflow_failed alertmanager_high_error_rate trade_rejected; do
  ingest < "$FIX/$f.json" && echo "  ok $f"
done
sleep 2

echo "== (b) x$DUPS duplikatów WORKFLOW_FAILED (jeden alert, rośnie licznik)"
seq 1 "$DUPS" | xargs -P 16 -I{} sh -c 'curl -sf -o /dev/null -X POST -H "Content-Type: application/json" --data-binary @"$FIX/workflow_failed.json" "$INGEST_URL/ingest"'
echo "  wysłano $DUPS"

echo "== (c) Settlements: nowy alert -> runbook z zatwierdzeniem (krok eskalacji)"
patch "$FIX/workflow_failed.json" "p['step']='demo-step-$RUN'; p['workflowId']='WF-DEMO-$RUN'" | ingest && echo "  ok (step demo-step-$RUN)"

if [ "$DEMO_OUTAGE" = 1 ]; then
  echo "== (d) awaria kanału SMS -> alert krytyczny payments (Anna: SMS w dzień) -> fallback"
  sms false
  patch "$FIX/alertmanager_high_error_rate.json" "p['labels']['region']='demo-$RUN'" | ingest && echo "  ok (region demo-$RUN)"
  sleep 8
  sms true
  echo "  kanał SMS przywrócony"
else
  echo "== (d) POMINIĘTE (DEMO_OUTAGE=0): awaria kanału SMS"
fi

echo "== (e) nieznany schemat -> UNMAPPED"
curl -sf -o /dev/null -X POST -H "Content-Type: application/json" -d "{\"host\":\"demo-$RUN\",\"level\":\"fatal\",\"msg\":\"disk full\"}" "$INGEST_URL/ingest/judges.custom.v1" && echo "  ok judges.custom.v1"

echo "== (f) runbook z nieznaną akcją (flush_cdn): krok ręczny, patrz docs/DEMO.md (runbooki ładowane tylko z config/runbooks)"
sleep 3

echo
echo "== Podsumowanie"
api "$API_URL/alerts" | python3 -c '
import json,sys
d=json.load(sys.stdin); a=d.get("items",d) if isinstance(d,dict) else d
print("  alerty:",len(a))
for x in sorted(a,key=lambda x:-(x.get("count") or x.get("occurrences") or 0))[:5]: print("   ",x.get("title"),"| licznik",x.get("count",x.get("occurrences")),"|",x.get("state"))' || echo "  (nie udało się odczytać /alerts)"
api "$API_URL/unmapped" | python3 -c '
import json,sys
d=json.load(sys.stdin); a=d.get("items",d) if isinstance(d,dict) else d
s={str(x.get("schema")) for x in a}
print("  UNMAPPED: %d schematów, judges.custom.v1 %s"%(len(s),"jest" if "judges.custom.v1" in s else "BRAK"))' || true
cat <<MSG

  UI:           http://localhost:5173
  Mailpit:      http://localhost:8025
  mock-targets: http://localhost:8090/calls
  SMS/telefon:  $API_URL/mock/outbox
MSG
