# Design: Unify P1–P4, Refactor & Code Review

## Kontekst

Po zarchiwizowaniu `alerting-service-mvp` i `alerting-service-hardening` projekt przechodzi z trybu „4 równoległe strumienie z twardymi barierami własności” w tryb **jednego implementera**. Decyzje architektoniczne D1–D15 (Kafka/Redpanda, Postgres bez Redisa, single writer dla `alerts` i `runbooks`, idempotencja D9, bezpieczeństwo akcji D11) pozostają w mocy; zmieniamy wyłącznie **organizację kodu i konfiguracji** w stronę prostoty i braku duplikacji.

## Decyzje refaktoryzacyjne

### R1. Jedna, płaska struktura `config/` (bez `p1/`, `p2/`, `p3/`)
Docelowe drzewko katalogu `config/`:
```
config/
  sources/      # źródła Kafki (dawniej config/p1/sources/)
  mappings/     # mappingi schematów (dawniej config/p1/mappings/)
  rules/        # reguły alertów (dawniej config/p2/rules/)
  people.yaml   # osoby, zespoły, preferencje (dawniej config/p2/people.yaml)
  runbooks/     # runbooki AUTOMATIC i MANUAL (dawniej config/p3/runbooks/)
  actions/      # akcje deklaratywne HTTP (dawniej config/p3/actions/)
  demo/         # dane i notatki do demo
  README.md
```
- Wszystkie ścieżki w `ingest`, `lifecycle`, `router`, `runbooks`, `api` (w tym `source_file` w `GET /people` i `GET /runbooks`), testach i `scripts/e2e/` zostają zaktualizowane na nową strukturę.
- Żaden plik w repozytorium nie odwołuje się już do `config/p1`, `config/p2` ani `config/p3`.

### R2. Domenowe moduły w `crates/api/src/` (zamiast `p1.rs`, `p2.rs`, `p3.rs`)
```
crates/api/src/
  main.rs          # start serwera, auth middleware, router()
  state.rs         # AppState (db, kafka, config)
  error.rs         # wspólny ApiError / ApiResult, db_err(), parse_id(), opt_json()
  rules_eval.rs    # współdzielony CompiledRule + load_enabled_rules() + enrich_alert() (dla alerts.rs i stream.rs!)
  stream.rs        # SSE /stream
  routes/
    mod.rs         # scala wszystkie domeny w jeden Router<AppState>
    alerts.rs      # /alerts, /alerts/{id}, /episodes, /ack, /resolve, /ingest/errors
    mappings.rs    # /mappings, /mappings/preview, /unmapped
    rules.rs       # /rules, /rules/{id}, /rules/test, /fields
    people.rs      # /people, /teams
    deliveries.rs  # /alerts/{id}/deliveries, /deliveries, /resend, /mock/*
    runbooks.rs    # /runbooks, /runbooks/errors, /alerts/{id}/runbooks, /runs/..., /approvals, /alerts/{id}/audit
    actions.rs     # /actions, /actions/{name}, /actions/missing
```
- Eliminuje duplikację `err()`, `db_err()`, `opt_json()`, `RuleTeams` vs `CompiledRule` oraz 900-liniowe monolity per osoba.

### R3. Wspólny wzorzec hot-reloadu konfiguracji w `crates/common`
- Dziś `lifecycle/src/config.rs`, `router/src/config.rs` i `runbooks/src/store.rs` powielają logikę listowania plików `*.yaml`, sprawdzania `mtime`, publikacji `UiEventKind::ConfigChanged` oraz nasłuchu `PgListener('config_changed')`.
- Wydzielamy minimalny, czytelny moduł `common::reload` (funkcje pomocnicze bez nadmiernych traitów/generyków), upraszczając wszystkie 3 crate'y.

### R4. Migracje bazodanowe
- `sqlx::migrate!` trzyma wersje w `_sqlx_migrations`, więc na istniejącej bazie nie zmieniamy nazw już zastosowanych plików bez `down -v`.
- Ponieważ docelowy start to czysty klon (`make demo` / `docker compose up`), możemy skonsolidować puste pliki startowe `1000_p1_init.sql`, `2000_p2_init.sql`, `3000_p3_init.sql` i ujednolicić nazwy domenowo (`0001_ingest_and_mappings.sql`, `0002_alerts.sql`, `0003_rules_people_deliveries.sql`, `0004_runbooks_and_actions.sql`), a następnie zweryfikować na czystym `docker compose down -v && make demo`.

### R5. Integracja refaktoru UI (`origin/p4/ui-refactor-1`)
- Scalenie `main` do `p4/ui-refactor-1` z zachowaniem wszystkich poprawek z `main`:
  - `ba0675b` (PR #13 `ui-fixes`),
  - `8c3ae0f` (PR #26 — `?token=` w SSE `/stream`),
  - `e534f71` (PR #29 — łańcuch fallbacku w `Deliveries.tsx`).
- Weryfikacja w kontenerze `ui` (`npm run typecheck`, `npm test`, `npm run build`) i merge do `main`.

### R6. Zasady upraszczania i Code Review
- Każdy refaktor wykonywany jest w małych, izolowanych krokach z pełną weryfikacją (`cargo test --workspace`, `cargo clippy --workspace --all-targets`, skrypty `scripts/e2e/*.sh`).
- Zakaz przedwczesnych abstrakcji: łączymy kod tylko tam, gdzie istnieje realna duplikacja (≥ 2 miejsca) lub wyraźny zysk czytelności.
- Każdy moduł przechodzi checklistę z `specs/code-review/spec.md` (logika, bezpieczeństwo, wydajność, testy, minimalność).
