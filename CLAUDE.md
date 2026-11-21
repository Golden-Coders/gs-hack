# CLAUDE.md — Alerting Service

Serwis alertingowy: zdarzenia w dowolnym schemacie → alerty z deduplikacją → reguły → powiadomienia na preferowanych kanałach → runbooki z akcjami i zatwierdzeniami. Zadanie: `docs/exercise.md`.

## Źródła prawdy (czytaj przed pracą)

Specyfikacja systemu i plan w OpenSpec:

| Gdzie | Co zawiera |
|---|---|
| `openspec/specs/<capability>/spec.md` | **aktualne wymagania i scenariusze** (= przypadki testowe) |
| `openspec/changes/unify-and-refactor/` | **aktywny change**: unifikacja struktury po P1–P4, uproszczenie codebase'u, code review i pozostałe weryfikacje |
| `openspec/changes/archive/` | zarchiwizowane `alerting-service-mvp` (design D1–D15) i `alerting-service-hardening` |

Implementację prowadź przez `/openspec-apply-change unify-and-refactor`. Odhaczaj zadania w `tasks.md` (`- [x]`) dopiero po spełnieniu weryfikacji opisanej w zadaniu.

## Tryb pracy (jeden implementer)

Projekt przeszedł z fazy 4 równoległych strumieni (P1–P4) w tryb **jednego implementera** — wszystkie katalogi (`crates/`, `config/`, `migrations/`, `ui/`, `scripts/`, `docs/`) tworzą jeden spójny codebase zorganizowany domenowo.

### Zasady techniczne
1. **Single writer:** stan alertu zmienia tylko `lifecycle`, stan runbooka tylko `runbooks`. Pozostałe serwisy wysyłają komendy na `alerts.commands` / `runbooks.commands`.
2. **Zależności Rust:** wspólne wersje w `[workspace.dependencies]`; w crate'cie `nazwa = { workspace = true }`.
3. **At-least-once:** offset Kafki commituj PO zapisaniu skutków. Każdy skutek uboczny musi być idempotentny (`contracts::keys`, atomowy `claim_delivery` w `sender`).
4. **Bezpieczeństwo akcji:** wykonujemy tylko akcje z rejestru; `risk: high` zawsze wymaga zatwierdzenia; allowlista hostów (`ACTION_HOST_ALLOWLIST`) bez redirectów poza nią; szablony w sandboxie bez dostępu do plików/env; sekrety tylko jako `${ENV}`.
5. **Kontrakt OpenAPI:** przy zmianie `contracts/openapi.yaml` zregeneruj typy w UI (`cd ui && npm run gen:api`).
6. **Język:** dokumenty i komentarze po polsku, identyfikatory w kodzie po angielsku.

## Architektura i struktura

```
HTTP /ingest(/{schema}) --+
                          +--> events.raw --> [lifecycle: normalizer] --> events.normalized (key=fingerprint)
Kafka (config/sources/) --+                         |  brak mappingu --> events.unmapped
                                                    v
                         alerts.commands --> [lifecycle: dedup + stan] --> alerts.changed
                                                                              |            \
                                                                          [router]      [runbooks] --> runbooks.commands (z UI)
                                                                              |              | escalate/notify
                                                                              v              v
                                                                     deliveries.requested --> [sender] --> UI / Email / SMS / Phone
                                                                                             retry --> deliveries.retry --> DLQ
lifecycle/router/sender/runbooks --> ui.events --> [api] --SSE /stream--> UI
```

Stos: Rust (tokio, axum, rdkafka, sqlx, minijinja), Kafka API (lokalnie Redpanda), Postgres 16, Mailpit (mock SMTP), UI: React + Vite + TypeScript. **Bez Redisa.**

- **Konfiguracja (`config/`):** `sources/`, `mappings/`, `rules/`, `people.yaml`, `runbooks/`, `actions/`, `demo/`. W runtime źródłem prawdy jest Postgres (`NOTIFY config_changed`).
- **Migracje (`migrations/`):** domenowe `0001_ingest_and_mappings.sql` … `0004_runbooks_and_actions.sql`, uruchamiane przy starcie przez `common::db::connect_and_migrate`.
- **API dla UI (`crates/api`):** port `:8080`, token `Authorization: Bearer dev-token` (lub `?token=dev-token` dla `/stream`), tożsamość w nagłówku `X-Person-Id`.

## Komendy

```bash
docker compose up -d --build              # całość: UI :5173, API :8080, Mailpit :8025, mock-targets :8090
make demo                                 # build + up + seed + scenariusz demo
docker compose down -v && make demo       # czyste środowisko od zera
cargo test --workspace                    # testy Rusta
cargo clippy --workspace --all-targets    # linter
scripts/e2e/first-flow.sh                 # e2e: pierwszy przepływ
scripts/e2e/appendix-a.sh                 # e2e: 3 schematy + dedup 500 + reopen
scripts/e2e/runbooks.sh                   # e2e: runbooki AUTOMATIC i MANUAL
RUN_RULE_CHANGE=1 scripts/e2e/criteria.sh # e2e: kryteria demo
scripts/e2e/resilience.sh                 # e2e: odporność (restarty + fallback + DLQ)
```
