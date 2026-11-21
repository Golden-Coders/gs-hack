#!/usr/bin/env bash
# Buduje obraz alerting-backend:loadtest z bieżącego kodu (nie rusza alerting-backend:dev).
set -euo pipefail
cd "$(dirname "$0")/.."
docker compose -p gsload -f docker-compose.yml -f loadtest/docker-compose.loadtest.yml build backend-build
