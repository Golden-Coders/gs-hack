# Proposal: Unify P1–P4, Refactor & Code Review

## Why

Oba wcześniejsze etapy (`alerting-service-mvp` i `alerting-service-hardening`) zostały zarchiwizowane — cały system działa end-to-end (500 msg/s, p50 123 ms bez email / 16.6 ms email, 0 duplikatów, odporność na restarty i awarie kanałów, czyste `make demo`).

Dotychczasowa struktura repozytorium była podporządkowana pracy równoległej 4 osób (`P1`–`P4`): osobne katalogi `config/p1/`, `config/p2/`, `config/p3/`, moduły `crates/api/src/p1.rs`, `p2.rs`, `p3.rs`, prefiksy `1xxx_p1_*` / `2xxx_p2_*` / `3xxx_p3_*` w migracjach, zduplikowany kod hot-reloadu i obsługi błędów między crate'ami oraz osobny, niedokończony branch `p4/ui-refactor-1`.

Od teraz nad projektem pracuje **jeden implementer**. Celem tego change'a jest:
1. **Połączenie rozbitych obszarów P1–P4 w jedną spójną, domenową strukturę** (bez sztucznych katalogów i nazw `p1`/`p2`/`p3`/`p4`).
2. **Maksymalne uproszczenie i uporządkowanie codebase'u** (usunięcie duplikacji, martwego kodu, komentarzy `shortcut:`, domknięcie refaktoru UI z `p4/ui-refactor-1`).
3. **Formalny przegląd kodu (Code Review)** całego repozytorium pod kątem poprawności, bezpieczeństwa, wydajności i minimalności.
4. **Domknięcie ostatnich pozostałych weryfikacji** przeniesionych z `alerting-service-hardening` (weryfikacja lagu `sender` z 2.4, weryfikacja ekranów UI/README 4.1–4.6 oraz próby generalne demo 5.4).

## What Changes

- **Unifikacja konfiguracji:** przeniesienie `config/p1/{sources,mappings}`, `config/p2/{rules,people.yaml}`, `config/p3/{runbooks,actions}` do płaskiej, domenowej struktury `config/{sources,mappings,rules,people.yaml,runbooks,actions,demo}` i aktualizacja ścieżek w `ingest`, `lifecycle`, `router`, `runbooks`, `api`, skryptach E2E oraz dokumentacji.
- **Unifikacja `crates/api`:** zastąpienie sztucznego podziału `p1.rs`/`p2.rs`/`p3.rs` domenowymi modułami (`routes/{alerts,mappings,rules,people,deliveries,runbooks,actions}.rs`), wspólnym modułem błędów (`error.rs`) i wspólnym ewaluatorem dopasowania reguł dla `GET /alerts` oraz SSE `/stream`.
- **Uproszczenie backendu Rust:** wydzielenie powtarzalnego mechanizmu ładowania/hot-reloadu konfiguracji do `crates/common`, usunięcie martwego kodu i `unwrap()` w ścieżkach produkcyjnych, przejście `cargo clippy --workspace --all-targets` bez ostrzeżeń, aktualizacja `CLAUDE.md` pod tryb jednego implementera.
- **Domknięcie refaktoru UI:** integracja `origin/p4/ui-refactor-1` z aktualnym `main` (z zachowaniem poprawek z PR #13, #26 i #29), zielone `npm run typecheck`, `vitest` i `npm run build`.
- **Code Review:** systematyczny przegląd wszystkich crate'ów i UI wg nowej specyfikacji `code-review`, naprawa znalezionych problemów.
- **Dokończenie weryfikacji hardeningu:** pomiar lagu 1 instancji współbieżnego `sender`a (dawne 2.4), weryfikacja UI/README na prawdziwym backendzie (dawne 4.1–4.6) i 2 próby generalne demo + nagranie ≤ 5 min (dawne 5.4).

## Capabilities

### New Capabilities
- `code-refactoring`: spójne zasady unifikacji struktury (usunięcie podziału P1–P4), deduplikacji, upraszczania kodu i zachowania kontraktów podczas refaktoryzacji.
- `code-review`: formalne kryteria przeglądu kodu (poprawność, bezpieczeństwo, wydajność pod obciążeniem, pokrycie testami, minimalność i brak zbędnych abstrakcji).

### Modified Capabilities
- Brak zmian zewnętrznego zachowania API/topików/UI (wszystkie istniejące specyfikacje `openspec/specs/*` pozostają w pełni spełnione).

## Impact

- Katalogi: `config/`, `crates/api/src/`, `crates/common/src/`, `crates/ingest/`, `crates/lifecycle/`, `crates/router/`, `crates/sender/`, `crates/runbooks/`, `ui/`, `migrations/`, `scripts/`, `CLAUDE.md`, `README.md`.
- Kontrakty zewnętrzne (`contracts/openapi.yaml`, topiki Kafki `crates/contracts`) pozostają niezmienione i wstecznie kompatybilne.
