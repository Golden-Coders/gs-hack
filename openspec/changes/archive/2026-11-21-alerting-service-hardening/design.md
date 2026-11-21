# Design

## Kontekst

Decyzje techniczne D1–D15 z `openspec/changes/archive/2026-11-21-alerting-service-mvp/design.md` obowiązują bez zmian. Ten change nie wprowadza nowych decyzji architektonicznych poza poniższymi ustaleniami wykonawczymi.

## Ustalenia

1. **Środowisko pomiarów i testów.** Wszystkie bramki z sekcji 2–3 wykonuje się na obrazach zbudowanych z aktualnego `main` (zapisać commit i czas budowy obrazów). Pomiar 1 był na obrazach z 00:30 (sprzed PR #26/#27), więc nie jest punktem odniesienia dla pomiaru końcowego bez tej adnotacji.
2. **Współdzielony stack.** Przebiegi obciążeniowe, przełączanie kanałów mock i zmiany reguł wymagają okna uzgodnionego z innymi sesjami/osobami (jak przy pomiarze 1: 02:10–02:40). Skrypty e2e muszą działać na niepustym systemie (unikalne fingerprinty) i 2× z rzędu.
3. **Czysty klon (`down -v`).** Tylko w uzgodnionym oknie; to jedyny sposób na zweryfikowanie 17.1, 18.6 i 19.H1.
4. **`sender` — optymalizacja (opcjonalna).** Przetwarzanie współbieżne per partycja (albo `buffer_unordered`) z commitem offsetów w kolejności i pulą połączeń SMTP; idempotencja nadal po `contracts::keys::delivery` (D9). Alternatywa bez zmian kodu: `--scale sender=N` (12 partycji). Decyzja po pomiarze G1.
5. **D10 (rozstrzygnięte w 6.1).** `lifecycle` pozostaje jedynym pisarzem `alerts` i nie zna reguł. `crates/api/src/p1.rs` czyta tabelę `rules` P2 tylko do odczytu (tak samo jak `router`, `runbooks` i SSE `/stream`), ewaluuje włączone reguły przez `condition` na `alerts.last_event` i zwraca `rule_ids`, `teams` (`rule.team` + odbiorcy typu `team`), najwyższe `severity_override`, `color`, `title`, `body` oraz `fields` (z `highlight`) zgodne z `contracts/openapi.yaml` i `openspec/specs/operator-console/spec.md`. Filtr `GET /alerts?team=<id>` filtruje po wyliczonych `teams`. Zachowanie realizuje istniejący kontrakt OpenAPI i specyfikację `operator-console` („Feed alertów na żywo” + „Szczegóły alertu”), więc nie wymaga zmiany specyfikacji.
