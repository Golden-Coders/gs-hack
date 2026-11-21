# Proposal

## Why

MVP `alerting-service-mvp` jest zarchiwizowane (`openspec/changes/archive/2026-11-21-alerting-service-mvp/`), a jego specyfikacje są głównymi specami w `openspec/specs/`. Kod wszystkich capabilities jest na `main`, ale 17 z 87 zadań MVP nie przeszło jeszcze weryfikacji opisanej w zadaniu (odporność, skalowanie, pomiar końcowy, przebiegi na czystym klonie, próby demo). Pomiar 1 (`docs/LOADTEST.md`) pokazał też wąskie gardło w `sender` (~200–260 dostarczeń/s na instancję). Ten change zbiera pozostałą pracę w jednym aktywnym planie, żeby zespół i agenci wiedzieli, co zostało przed demo.

## What Changes

- Weryfikacja end-to-end tego, co jest już zaimplementowane: odporność (restart binarek, awaria Mailpit/SMS), skalowanie (`lifecycle` i `sender` ×2), kryteria demo na czystym klonie, próby generalne nagrania.
- Pomiar końcowy wydajności w `docs/LOADTEST.md` (na obrazach zbudowanych z aktualnego `main`).
- Weryfikacja na prawdziwym backendzie ekranów UI, których kod jest zmergowany (kolejka zatwierdzeń, podgląd mocków i fallback, edytory reguł/mappingów/akcji, README).
- Opcjonalnie: współbieżne przetwarzanie i pula połączeń SMTP w `sender` — bez zmiany zachowania z punktu widzenia specyfikacji (`notification-delivery`).
- Domknięcie otwartych kwestii: `rule_ids`/`teams`/severity reguły w `GET /alerts` (design D10) i różnica 3 alertów względem unikalnych fingerprintów w pomiarze 1.

## Capabilities

### New Capabilities

Brak.

### Modified Capabilities

Brak zmian wymagań — change dotyczy weryfikacji, wydajności i dokumentacji (`skip_specs: true`). Jeśli rozwiązanie D10 zmieni zachowanie `GET /alerts`, dodać delta spec dla `alert-lifecycle` / `operator-console`.

## Impact

- Skrypty: `scripts/e2e/` (nowe `resilience.sh`, rozszerzenia istniejących), `scripts/demo/`, `Makefile`.
- Dokumentacja: `docs/LOADTEST.md`, `docs/DEMO.md`, `README.md`.
- Kod (tylko przy optymalizacji i D10): `crates/sender`, ewentualnie `crates/lifecycle`, `crates/api/src/p1.rs`.
- Środowisko: przebiegi wymagają przebudowy stacku z aktualnego `main` (`docker compose up -d --build`); testy na czystym klonie wymagają `docker compose down -v` — uzgodnić z zespołem, bo stack bywa współdzielony.
