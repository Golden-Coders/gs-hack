#!/usr/bin/env bash
# Lag konsumentów per grupa (D14, sekcja 5.2). Dowód, gdzie jest wąskie gardło.
# Użycie: crates/loadgen/lag.sh [grupa ...]   (bez argumentów – domyślna lista)
# Kontener brokera: REDPANDA_CONTAINER (domyślnie gs-hack-redpanda-1); `docker exec`,
# a nie `docker compose exec`, więc działa z dowolnego katalogu (także z git worktree).
# Kod wyjścia ≠ 0, gdy nie udało się opisać żadnej grupy (np. zły kontener).
set -uo pipefail

CONTAINER="${REDPANDA_CONTAINER:-gs-hack-redpanda-1}"

if [ "$#" -gt 0 ]; then
    CONSUMER_GROUPS=("$@")
else
    # Uwaga: nie `GROUPS` — to tylko-do-odczytu zmienna basha (GID-y), przypisanie jest ignorowane.
    CONSUMER_GROUPS=(lifecycle-normalizer lifecycle-dedup router runbooks sender sender-retry ingest-source-trades)
fi

ok=0
for group in "${CONSUMER_GROUPS[@]}"; do
    echo "=== $group"
    if docker exec "$CONTAINER" rpk group describe "$group"; then
        ok=$((ok + 1))
    else
        echo "(grupa $group niedostępna w kontenerze $CONTAINER)" >&2
    fi
    echo
done

if [ "$ok" -eq 0 ]; then
    echo "BŁĄD: nie udało się opisać żadnej grupy (kontener $CONTAINER działa? ustaw REDPANDA_CONTAINER)" >&2
    exit 1
fi
echo "Opisano $ok z ${#CONSUMER_GROUPS[@]} grup."
