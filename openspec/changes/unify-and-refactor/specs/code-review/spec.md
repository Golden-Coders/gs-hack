# code-review Specification

## Purpose

Definiuje formalne kryteria przeglądu kodu (Code Review) stosowane przy porządkowaniu całego codebase'u oraz przy każdej kolejnej zmianie — nastawione na poprawność, bezpieczeństwo, wydajność pod obciążeniem, testy i prostotę.

## ADDED Requirements

### Requirement: Poprawność logiczna i odporność na błędy
Każdy przeglądany moduł SHALL być zweryfikowany pod kątem warunków brzegowych, idempotencji (D9), kolejności commitowania offsetów Kafki (at-least-once) oraz braku panik (`unwrap()`/`expect()`/`panic!()`) w ścieżkach produkcyjnych poza inicjalizacją i testami.

#### Scenario: Brak niekontrolowanych panik na danych wejściowych
- **WHEN** przegląd kodu wykryje `.unwrap()` lub `.expect()` wywoływane na danych z HTTP, Kafki lub bazy danych w kodzie produkcyjnym
- **THEN** zostaje zastąpione jawną obsługą błędu (`Result` / `Option`) z logowaniem i testem przypadku brzegowego

#### Scenario: Weryfikacja idempotencji i at-least-once
- **WHEN** konsument Kafki wykonuje operację ze skutkiem ubocznym (zapis do bazy, wysyłka powiadomienia, wywołanie akcji HTTP)
- **THEN** commit offsetu następuje wyłącznie po trwałym zapisie skutku, a ponowne przetworzenie tej samej wiadomości nie powoduje duplikatu

### Requirement: Bezpieczeństwo (Security Review)
Przegląd kodu SHALL weryfikować wszystkie granice bezpieczeństwa systemu: uwierzytelnianie API/SSE, sandbox szablonów `condition`, allowlistę hostów i brak przekierowań poza nią w akcjach HTTP, wymuszanie `approval` dla `risk: high` oraz brak wycieku sekretów `${ENV}` w odpowiedziach API i logach.

#### Scenario: Brak wycieku sekretów i SSRF
- **WHEN** przegląd obejmuje `crates/runbooks`, `crates/condition` i `crates/api`
- **THEN** potwierdzono brak rozwijania `${ENV}` w odpowiedziach GET API, blokadę przekierowań HTTP poza `ACTION_HOST_ALLOWLIST`, wymuszenie zatwierdzenia dla `risk: high` oraz brak obejścia uwierzytelniania Bearer/`?token=`

### Requirement: Wydajność i zachowanie pod obciążeniem
Przegląd kodu SHALL identyfikować wąskie gardła w ścieżce krytycznej (brak N+1 zapytań SQL na listach, brak blokowania wątków Tokio operacjami synchronicznymi `std::fs`/`std::thread::sleep` w async, ograniczone rozmiary cache'y w pamięci).

#### Scenario: Brak zapytań N+1 i nieograniczonych cache'y
- **WHEN** przegląd obejmuje `crates/api`, `crates/lifecycle`, `crates/router`, `crates/sender`, `crates/runbooks`
- **THEN** każdy endpoint listujący wykonuje stałą liczbę zapytań SQL (niezależną od N wyników), a każda mapa w pamięci ma jasny limit rozmiaru lub cykl czyszczenia

### Requirement: Minimalność, czytelność i pokrycie testami
Przegląd kodu SHALL oceniać każdą istotną ścieżkę pod kątem prostoty (brak zbędnych warstw, kopiuj-wklej i nieużywanych flag) oraz pokrycia testami dla logiki wysokiego ryzyka (ewaluator, dedup, routing, idempotencja sendera, bezpieczeństwo akcji).

#### Scenario: Raport przeglądu z rankingiem ważności
- **WHEN** przeprowadzany jest przegląd modułu lub całego repozytorium
- **THEN** znaleziska są klasyfikowane według wpływu (krytyczne / ważne / uproszczenie), a wszystkie krytyczne i ważne są naprawiane z testem regresyjnym przed zamknięciem zadania
