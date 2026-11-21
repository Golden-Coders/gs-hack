#!/usr/bin/env bash
# Lag konsumentów per grupa (D14, sekcja 5.2). Dowód, gdzie jest wąskie gardło.
# Użycie: crates/loadgen/lag.sh [grupa ...]   (bez argumentów – domyślna lista)
# Z innego katalogu niż główny klon (np. git worktree) ustaw COMPOSE_PROJECT_NAME=gs-hack,
# inaczej docker compose szuka projektu o nazwie katalogu.
set -euo pipefail

cd "$(dirname "$0")/../.."

if [ "$#" -gt 0 ]; then
    CONSUMER_GROUPS=("$@")
else
    # Uwaga: nie `GROUPS` — to tylko-do-odczytu zmienna basha (GID-y), przypisanie jest ignorowane.
    CONSUMER_GROUPS=(lifecycle-normalizer lifecycle-dedup router runbooks sender sender-retry ingest-source-trades)
fi

for group in "${CONSUMER_GROUPS[@]}"; do
    echo "=== $group"
    docker compose exec -T redpanda rpk group describe "$group" || echo "(grupa $group niedostępna)"
    echo
done
