#!/usr/bin/env bash
# Czysty stos testowy `gsload` (down -v + up). Użycie: loadtest/reset.sh [--scale usługa=N ...]
# Np.: loadtest/reset.sh --scale lifecycle=4 --scale sender=2
set -euo pipefail
cd "$(dirname "$0")/.."
C="docker compose -p gsload -f docker-compose.yml -f loadtest/docker-compose.loadtest.yml"
$C down -v >/dev/null 2>&1 || true
$C up -d "$@" >/dev/null 2>&1
for _ in $(seq 1 60); do
  n=$(docker exec gsload-postgres-1 psql -U alerting -d alerting -tAc "select count(*) from rules" 2>/dev/null || echo 0)
  [ "${n:-0}" -gt 0 ] && break; sleep 2
done
sleep 12   # dołączenie konsumentów do grup (rebalance)
echo "stos gsload gotowy: $($C ps --format '{{.Service}}' | sort | uniq -c | tr '\n' ' ')"
