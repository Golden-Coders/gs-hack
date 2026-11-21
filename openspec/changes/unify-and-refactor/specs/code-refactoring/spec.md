# code-refactoring Specification

## Purpose

Określa zasady porządkowania, unifikacji i upraszczania codebase'u po przejściu z 4 równoległych strumieni (P1–P4) na jednego implementera, bez regresji w działających funkcjonalnościach i kontraktach.

## ADDED Requirements

### Requirement: Jedna spójna struktura domenowa bez podziału P1–P4
Repozytorium SHALL organizować konfigurację, moduły API, migracje i dokumentację według domen biznesowych (`sources`, `mappings`, `alerts`, `rules`, `people`, `deliveries`, `runbooks`, `actions`), a nie według numerów osób (`p1`, `p2`, `p3`, `p4`).

#### Scenario: Brak katalogów i modułów p1/p2/p3 w aktywnym kodzie
- **WHEN** wyszukujemy ścieżki `config/p1`, `config/p2`, `config/p3` lub moduły `crates/api/src/p1.rs`, `p2.rs`, `p3.rs`
- **THEN** żadna z tych ścieżek nie istnieje w aktywnym kodzie ani konfiguracji, a wszystkie binarki ładują konfigurację z płaskiego katalogu `config/`

#### Scenario: Spójne ścieżki źródłowe w API i UI
- **WHEN** użytkownik wywołuje `GET /people` lub `GET /runbooks`
- **THEN** pole `source_file` wskazuje rzeczywistą, ujednoliconą ścieżkę (np. `config/people.yaml`, `config/runbooks/payments-high-error-rate.md`)

### Requirement: Eliminacja duplikacji między modułami
Wspólne mechanizmy powielone między strumieniami (obsługa błędów HTTP w `api`, ewaluacja reguł na `CanonicalEvent` dla `GET /alerts` i SSE `/stream`, wzorce ładowania/hot-reloadu plików YAML) SHALL być wydzielone do jednego miejsca bez wprowadzania zbędnych warstw abstrakcji.

#### Scenario: Jedna logika dopasowania reguł w api
- **WHEN** `GET /alerts?team=<id>` i SSE `/stream?team=<id>` wyznaczają zespoły dla tego samego alertu
- **THEN** oba korzystają z tej samej funkcji w `crates/api/src/rules_eval.rs` i zwracają identyczny zestaw zespołów

#### Scenario: Wspólny moduł błędów w api
- **WHEN** dowolny handler w `crates/api/src/routes/` zwraca błąd HTTP lub DB
- **THEN** używa wspólnego `ApiError` z `crates/api/src/error.rs` zamiast lokalnej kopii `err()` / `db_err()`

### Requirement: Uproszczenie kodu i brak martwego kodu
Refaktoryzacja SHALL zmniejszać złożoność i liczbę zbędnych linii: usuwać nieużywane funkcje, stare zaślepki z Fazy 0, nieaktualne komentarze `shortcut:`/`TODO` oraz nadmiernie rozbudowane funkcje.

#### Scenario: Czysta kompilacja i linter
- **WHEN** uruchamiamy `cargo clippy --workspace --all-targets` oraz `npm run typecheck` w `ui/`
- **THEN** oba polecenia kończą się kodem `0` bez ostrzeżeń o martwym kodzie, nieużywanych importach ani błędach typów

### Requirement: Zachowanie kontraktów i brak regresji
Każdy krok refaktoryzacji SHALL zachowywać w pełni kontrakty z `contracts/openapi.yaml`, `crates/contracts` oraz wszystkie wymagania z `openspec/specs/`.

#### Scenario: Zielona regresja po refaktoryzacji
- **WHEN** po zakończonej unifikacji uruchamiamy testy jednostkowe (`cargo test --workspace`, `npm test` w `ui/`) oraz skrypty E2E (`first-flow.sh`, `appendix-a.sh`, `runbooks.sh`, `criteria.sh`, `resilience.sh`) i czyste `docker compose down -v && make demo`
- **THEN** wszystkie testy i scenariusze kończą się sukcesem bez zmian w kontraktach API
