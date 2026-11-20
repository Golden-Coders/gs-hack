#!/usr/bin/env bash
# Lag konsumentów per grupa (D14, sekcja 5.2). Dowód, gdzie jest wąskie gardło.
# Użycie: crates/loadgen/lag.sh [grupa ...]   (bez argumentów – domyślna lista)
set -euo pipefail

cd "$(dirname "$0")/../.."

if [ "$#" -gt 0 ]; then
    GROUPS=("$@")
else
    GROUPS=(lifecycle-normalizer lifecycle-dedup router runbooks sender sender-retry ingest-source-trades)
fi

for group in "${GROUPS[@]}"; do
    echo "=== $group"
    docker compose exec -T redpanda rpk group describe "$group" || echo "(grupa $group niedostępna)"
    echo
done
