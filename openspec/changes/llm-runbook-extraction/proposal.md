# Proposal: Ekstrakcja kroków runbooka przez LLM (klient OpenAI-compatible)

## Why

Obecnie runbooki w formacie Markdown lub wolnego tekstu (`.md`, `.txt`) są wyłącznie runbookami `MANUAL` — system wyświetla je jako tekst i nie potrafi zaproponować na ich podstawie kroków wykonywalnych (`AUTOMATIC`) z akcjami z rejestru. Jednocześnie decyzja projektowa **D12** z MVP przygotowała już punkt rozszerzenia (`trait RunbookAuthor` w `crates/runbooks/src/author.rs` z dostawcą `none` i zmienną `LLM_PROVIDER`), odkładając właściwą implementację na osobny change.

Dodanie opcjonalnej ekstrakcji przez LLM pozwala operatorowi zamienić istniejący runbook `MANUAL` albo wklejony opis tekstowy w ustrukturyzowany szkic runbooka `AUTOMATIC` z podglądem i akceptacją człowieka — bez ręcznego pisania YAML, przy pełnym zachowaniu zasad bezpieczeństwa (redakcja danych wrażliwych przed wysłaniem, whitelista akcji z rejestru, wymuszone zatwierdzenia dla `risk: high`, wywołania poza ścieżką krytyczną alertu).

## What Changes

- **Wspólny klient OpenAI-compatible dla `RunbookAuthor`:** implementacja jednego klienta HTTP (`/v1/chat/completions` ze Structured Outputs / `json_schema`) obsługującego zarówno `LLM_PROVIDER=local` (domyślnie Ollama: `http://localhost:11434/v1`), jak i `LLM_PROVIDER=api` (np. OpenAI, Gemini OpenAI-compat, OpenRouter), obok dotychczasowego `LLM_PROVIDER=none` (domyślnego).
- **Redakcja danych wrażliwych przed wysłaniem do modelu:** deterministyczny moduł redakcji zamieniający adresy IP, e-maile, URL-e i hosty wewnętrzne na placeholdery (np. `[REDACTED_URL_1]`), przy zachowaniu zmiennych szablonowych `{{...}}`, tak aby do modelu trafiał wyłącznie zredagowany opis runbooka (nigdy dane alertów ani sekrety).
- **Walidacja i klasyfikacja szkicu po stronie backendu:** dynamiczne budowanie JSON Schema z polem `action` ograniczonym do enuma akcji z bieżącego rejestru (`actions`), parsowanie odpowiedzi do `RunbookDef` oraz obowiązkowe przepuszczenie przez `Registry::classify` (nieznana akcja degraduje do `manual`, `risk: high` zawsze wymusza `approval: required`).
- **Endpointy API (poza ścieżką alertu):**
  - `GET /runbooks/author/status` — zwraca, czy ekstrakcja LLM jest włączona (`provider`, `model`, `enabled`), aby UI mogło wyświetlić odpowiedni stan.
  - `POST /runbooks/draft` — przyjmuje tekst/Markdown (albo `runbook_id` istniejącego runbooka `MANUAL`), wykonuje redakcję, wywołuje LLM, klasyfikuje kroki wg rejestru i zwraca szkic (`RunbookDef`, podgląd YAML, listę zredagowanych tokenów i ostrzeżenia) **bez zapisywania**.
  - `POST /runbooks` — zapisuje zaakceptowany przez człowieka runbook `AUTOMATIC` w tabeli `runbooks` (`source_file = NULL`, analogicznie do akcji dodawanych z UI; hot reload w `runbooks` automatycznie włącza go do aktywnej migawki).
- **UI (ekran Runbooki):**
  - Przycisk „Wyodrębnij kroki (LLM)” przy runbookach `MANUAL` oraz panel „Importuj z tekstu (LLM)” do wklejenia Markdownu/tekstu.
  - Widok podglądu szkicu: lista wyodrębnionych kroków (z widocznym `automation`, `approval`, akcją i parametrami), informacja o zredagowanych tokenach, podgląd YAML z przyciskiem „Kopiuj YAML” oraz przycisk „Zapisz runbook” (akceptacja człowieka).
  - Gdy `LLM_PROVIDER=none` — wyraźna informacja w UI bez blokowania pozostałych funkcji.
- **Konfiguracja i dokumentacja:** opcjonalne zmienne `LLM_PROVIDER`, `LLM_BASE_URL`, `LLM_MODEL`, `LLM_API_KEY`, `LLM_TIMEOUT_MS` w `docker-compose.yml`, aktualizacja `contracts/openapi.yaml`, `docs/MOCKS.md` i `README.md`.

## Capabilities

### New Capabilities
- Brak nowych odrębnych capabilities — funkcja rozszerza istniejące capabilities `runbooks` i `operator-console`.

### Modified Capabilities
- `runbooks`: dodanie opcjonalnej ekstrakcji kroków z runbooków `MANUAL`/tekstu przez `RunbookAuthor` (OpenAI-compatible `local`/`api` + `none`), redakcji danych wrażliwych, ograniczenia schematem rejestru akcji oraz zapisu zaakceptowanego szkicu.
- `operator-console`: dodanie w ekranie Runbooki przepływu ekstrakcji szkicu przez LLM z podglądem, redakcją i jawną akceptacją operatora (oraz informacją o stanie dostawcy LLM).

## Impact

- Backend Rust: `crates/runbooks` (wydzielenie biblioteki `src/lib.rs` udostępniającej `author`, `redact`, `registry`, `parse`), `crates/api` (zależność od `runbooks`, nowe trasy w `routes/runbooks.rs` i stan `RunbookAuthor` w `AppState`), `crates/common` (opcjonalna konfiguracja zmiennych `LLM_*`).
- Kontrakty: addytywne rozszerzenie `contracts/openapi.yaml` o `/runbooks/author/status`, `/runbooks/draft` i `POST /runbooks` (bez zmian łamiących istniejące endpointy ani topiki Kafki).
- Frontend: `ui/src/screens/Runbooks.tsx`, regeneracja `ui/src/api/schema.d.ts`, testy jednostkowe UI.
- Infrastruktura i dokumentacja: `docker-compose.yml`, `docs/MOCKS.md`, `README.md`.
- Ścieżka krytyczna alertu (`ingest -> lifecycle -> router -> sender -> runbooks engine`) pozostaje **w 100% nietknięta** — brak wywołań LLM podczas przetwarzania alertów.
