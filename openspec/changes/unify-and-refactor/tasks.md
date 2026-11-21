# Tasks: Unify P1–P4, Refactor & Code Review

Nowy aktywny change po zarchiwizowaniu `alerting-service-mvp` i `alerting-service-hardening`. Realizowany przez **jednego implementera** (bez barier P1–P4). Odhaczać (`- [x]`) dopiero po spełnieniu weryfikacji opisanej w zadaniu.

## 1. Unifikacja struktury konfiguracji i dokumentacji (usunięcie podziału P1–P4)

- [x] 1.1 Przenieść konfigurację z `config/p1/`, `config/p2/`, `config/p3/` do płaskiej struktury domenowej `config/{sources,mappings,rules,people.yaml,runbooks,actions,demo}` i usunąć puste katalogi `p1/`, `p2/`, `p3/`; weryfikacja: `find config -maxdepth 2` nie zawiera żadnych katalogów `p1`/`p2`/`p3`
- [x] 1.2 Zaktualizować ścieżki ładowania konfiguracji i `source_file` w `crates/ingest`, `crates/lifecycle`, `crates/router`, `crates/runbooks`, `crates/api`, testach jednostkowych oraz `scripts/e2e/` i `scripts/demo/`; weryfikacja: `grep -rn 'config/p[123]' crates/ scripts/` zwraca 0 wyników, `cargo test --workspace` przechodzi
- [x] 1.3 Skonsolidować migracje `migrations/` w spójną domenową numerację (usunięcie pustych `1000_p1_init.sql`, `2000_p2_init.sql`, `3000_p3_init.sql` i prefiksów `p1`/`p2`/`p3`); weryfikacja: `docker compose down -v && docker compose up -d --build` nakłada wszystkie migracje bez błędów
- [x] 1.4 Zaktualizować `config/README.md`, `CLAUDE.md` i `README.md` pod tryb jednego implementera i nową płaską strukturę katalogów (bez zamrożonych barier P1–P4); weryfikacja: dokumentacja odpowiada rzeczywistym ścieżkom w repozytorium

## 2. Unifikacja i uproszczenie `crates/api`

- [ ] 2.1 Wydzielić wspólny `crates/api/src/error.rs` (`ApiError`/`ApiResult`, `db_err`, `parse_id`, `opt_json`) oraz `crates/api/src/rules_eval.rs` (wspólny `CompiledRule`, `load_enabled_rules`, `enrich_alert`, `teams_for_event` współdzielony między `GET /alerts` i SSE `/stream`); weryfikacja: `stream.rs` i `alerts` używają jednej implementacji dopasowania reguł, testy `api` zielone
- [ ] 2.2 Rozbić `crates/api/src/p1.rs`, `p2.rs`, `p3.rs` na spójne moduły domenowe `crates/api/src/routes/{mod,alerts,mappings,rules,people,deliveries,runbooks,actions}.rs` i usunąć `p1.rs`/`p2.rs`/`p3.rs`; weryfikacja: wszystkie trasy z `contracts/openapi.yaml` działają identycznie, `cargo test -p api` przechodzi
- [ ] 2.3 Uprościć `crates/api/src/main.rs` i `state.rs` (usunięcie starych komentarzy o zamrożeniu Fazy 0, jednoznaczne podpięcie `routes::router()`); weryfikacja: `cargo check -p api` bez ostrzeżeń

## 3. Uproszczenie i deduplikacja backendu Rust

- [ ] 3.1 Wydzielić wspólne pomocniki ładowania/hot-reloadu YAML i `UiEventKind::ConfigChanged` do `crates/common/src/reload.rs` i uprościć `crates/lifecycle/src/config.rs`, `crates/router/src/config.rs`, `crates/runbooks/src/store.rs`; weryfikacja: brak zduplikowanej logiki skanowania plików/mtime między crate'ami, testy wszystkich 3 crate'ów zielone
- [ ] 3.2 Usunąć martwy kod, nieużywane importy, stare zaślepki i przestarzałe komentarze `shortcut:` we wszystkich crate'ach; weryfikacja: `cargo clippy --workspace --all-targets -- -D warnings` przechodzi czysto
- [ ] 3.3 Przejrzeć zależności w `Cargo.toml` (workspace + wszystkie crate'y) i usunąć nieużywane zależności; weryfikacja: `cargo check --workspace` i `cargo test --workspace` przechodzą

## 4. Domknięcie refaktoru UI (`origin/p4/ui-refactor-1`)

- [ ] 4.1 Zintegrować `origin/p4/ui-refactor-1` z aktualnym `main`, zachowując wszystkie poprawki z `main` (PR #13 `ui-fixes`, PR #26 `?token=` w SSE `/stream`, PR #29 łańcuch fallbacku w `Deliveries.tsx`); weryfikacja: brak regresji w SSE i widoku dostarczeń
- [ ] 4.2 Zaktualizować ewentualne ścieżki `config/p*` wyświetlane w UI (`People`, `Runbooks`, testy) na nową strukturę `config/`; weryfikacja: `npm run typecheck`, `npm test` (vitest) i `npm run build` w kontenerze `ui` kończą się kodem 0
- [ ] 4.3 Zmergować zrefaktorowane UI do `main` i zweryfikować na żywym stacku (`http://localhost:5173`); weryfikacja: UI ładuje się bez błędów konsoli i przechodzi pełny scenariusz `make demo`

## 5. Systematyczny Code Review (wg `specs/code-review/spec.md`)

- [ ] 5.1 Przegląd poprawności i błędów: usunąć wszystkie produkcyjne `.unwrap()`/`.expect()` na danych zewnętrznych (HTTP/Kafka/DB), zweryfikować kolejność commitów offsetów Kafki i idempotencję w `ingest`, `lifecycle`, `router`, `sender`, `runbooks`; weryfikacja: raport przeglądu + testy regresyjne dla znalezionych przypadków
- [ ] 5.2 Przegląd bezpieczeństwa: zweryfikować uwierzytelnianie Bearer/`?token=`, sandbox `condition` (brak obejść), `ACTION_HOST_ALLOWLIST` + brak redirectów poza allowlistę, wymuszone `approval` dla `risk: high`, brak wycieku `${ENV}` w API i logach; weryfikacja: wszystkie testy bezpieczeństwa zielone
- [ ] 5.3 Przegląd wydajności i pamięci: sprawdzić zapytania N+1 w `crates/api/src/routes/`, brak blokującego I/O w zadaniach Tokio, limity pamięci map w `lifecycle`/`api`/`sender`; weryfikacja: brak krytycznych znalezisk, poprawki z testami
- [ ] 5.4 Pełna regresja E2E po refaktoryzacji i review: `first-flow.sh`, `appendix-a.sh`, `runbooks.sh`, `RUN_RULE_CHANGE=1 criteria.sh`, `resilience.sh` oraz czyste `docker compose down -v && make demo`; weryfikacja: wszystkie skrypty kończą się `PASS`

## 6. Pozostałe weryfikacje przeniesione z `alerting-service-hardening`

- [ ] 6.1 [hardening 2.4] Zweryfikować na przebudowanym obrazie współbieżnego `sender`a (1 instancja) przy 500 msg/s, że lag `sender` nie rośnie i wynik dopisać do `docs/LOADTEST.md`; weryfikacja: `cargo test -p sender` zielone, max lag `sender` stabilny przy 500 msg/s
- [ ] 6.2 [hardening 4.1] Weryfikacja UI na prawdziwym backendzie — kolejka zatwierdzeń: zatwierdzenie usuwa krok z kolejki i aktualizuje szczegóły alertu
- [ ] 6.3 [hardening 4.2] Weryfikacja UI na prawdziwym backendzie — podgląd mocków SMS/telefonu z przełącznikiem: wyłączenie SMS pokazuje fallback w szczegółach alertu
- [ ] 6.4 [hardening 4.3] Weryfikacja UI na prawdziwym backendzie — edytor reguł: utworzenie reguły w kreatorze bez pisania YAML
- [ ] 6.5 [hardening 4.4] Weryfikacja UI na prawdziwym backendzie — mappingi i UNMAPPED: nowy schemat → alert w feedzie w < 2 min
- [ ] 6.6 [hardening 4.5] Weryfikacja UI na prawdziwym backendzie — formularz akcji HTTP: dodanie `flush_cdn` zmienia krok runbooka `cdn-stale` na automatyczny
- [ ] 6.7 [hardening 4.6] Weryfikacja `README.md`: uruchomienie demo ze świeżego klona dokładnie wg instrukcji
- [ ] 6.8 [hardening 5.4] Próby generalne wg `docs/DEMO.md`: 2 przebiegi bez ingerencji i nagranie wideo ≤ 5 min
