# Tasks: Ekstrakcja kroków runbooka przez LLM (OpenAI-compatible)

## 1. Biblioteka `crates/runbooks` i moduł redakcji

- [x] 1.1 Utworzyć `crates/runbooks/src/lib.rs` eksportujący `author`, `parse`, `redact`, `registry` oraz zaktualizować `crates/runbooks/src/main.rs`, aby korzystał z modułów biblioteki bez duplikacji.
- [x] 1.2 Zaimplementować `crates/runbooks/src/redact.rs`: deterministyczna redakcja URL-i, e-maili, IPv4 i wewnętrznych nazw hostów (`.internal`, `.corp`, `.local`, `.svc`, `.cluster.local`) z zachowaniem zmiennych szablonowych `{{...}}` oraz zwracaniem raportu (`RedactedText`).
- [x] 1.3 Napisać testy jednostkowe dla `redact.rs` (URL, IP, e-mail, host wewnętrzny, zachowanie `{{raw.workflowId}}`, brak fałszywych trafień dla nazw usług typu `payments-api`).

## 2. Klient OpenAI-compatible w `crates/runbooks/src/author.rs`

- [x] 2.1 Rozszerzyć `RunbookAuthor` do asynchronicznego interfejsu (`draft(&self, input: &DraftRequest, actions: &[ActionDef]) -> Result<DraftOutcome, AuthorError>`) oraz dodać metody pomocnicze (`name()`, `model()`, `is_enabled()`).
- [x] 2.2 Zaimplementować `OpenAiCompatAuthor` korzystający z `reqwest` (`POST {base_url}/chat/completions`): budowanie promptu systemowego z opisem akcji z rejestru, dynamiczny JSON Schema (z `action` jako enumem akcji z rejestru + `null`), parsowanie odpowiedzi (w tym czyszczenie bloków ```` ```json ````), normalizacja `step.id`, klasyfikacja przez `Registry::classify` i zbieranie ostrzeżeń (`warnings`).
- [x] 2.3 Zaktualizować `author::from_env()`: obsługa `LLM_PROVIDER=none|local|api`, `LLM_BASE_URL`, `LLM_MODEL`, `LLM_API_KEY` (wymagany dla `api`), `LLM_TIMEOUT_MS` (domyślnie `30000`), jawny błąd przy nieznanej wartości `LLM_PROVIDER` lub brakującym `LLM_API_KEY`.
- [x] 2.4 Dodać serializację szkicu `RunbookDef` do czytelnego YAML zgodnego z `parse::parse_runbook` (podgląd i kopiowanie w UI).
- [x] 2.5 Napisać testy jednostkowe i integracyjne (z lokalnym mock serwerem `axum` w `dev-dependencies` `crates/runbooks`): poprawna ekstrakcja, weryfikacja że do mock serwera trafia zredagowany tekst, degradacja halucynowanej akcji spoza rejestru do `manual`, wymuszenie `approval: required` dla `risk: high`, obsługa błędu HTTP / timeoutu / niepoprawnego JSON-a, zachowanie `LLM_PROVIDER=none`.

## 3. Endpointy API i kontrakt OpenAPI

- [x] 3.1 Dodać `runbooks = { path = "../runbooks" }` do `crates/api/Cargo.toml` i inicjalizować `Arc<dyn RunbookAuthor>` w `AppState` (`crates/api/src/state.rs`).
- [x] 3.2 Dodać w `crates/api/src/routes/runbooks.rs`:
  - `GET /runbooks/author/status` — zwraca `{ enabled, provider, model }`,
  - `POST /runbooks/draft` — wczytuje bieżący rejestr akcji z bazy (i opcjonalnie treść runbooka `MANUAL` po `runbook_id`), wywołuje `RunbookAuthor::draft`, zwraca szkic, YAML, raport redakcji i ostrzeżenia (lub `501`/`400`/`404`/`502`/`504`),
  - `POST /runbooks` — waliduje zaakceptowany `RunbookDef`, blokuje nadpisanie runbooka z pliku (`409 Conflict`), klasyfikuje przez `Registry::classify`, zapisuje do tabeli `runbooks` (`source_file = NULL`) i zwraca `201 Created`.
- [x] 3.3 Zaktualizować `contracts/openapi.yaml` o nowe endpointy i schematy (`RunbookAuthorStatus`, `RunbookDraftRequest`, `RunbookDraftResponse`) oraz dodać testy tras w `crates/api/src/routes/runbooks.rs`.

## 4. Interfejs użytkownika (UI)

- [x] 4.1 Zregenerować typy TypeScript (`ui/src/api/schema.d.ts`) na podstawie `contracts/openapi.yaml`.
- [x] 4.2 Rozszerzyć `ui/src/screens/Runbooks.tsx` (i `Runbooks.css`):
  - pobranie `GET /runbooks/author/status` i wyświetlenie informacji o stanie LLM (aktywny dostawca/model lub komunikat, gdy `LLM_PROVIDER=none`),
  - przycisk „Wyodrębnij kroki (LLM)” przy runbookach `MANUAL` oraz panel „Importuj z tekstu (LLM)” (pole na tekst/Markdown, `id`, `title`),
  - widok podglądu szkicu: edycja `id`/`title`, lista wyodrębnionych kroków z odznakami (`automatyczny`/`ręczny`, `wymaga zatwierdzenia`), ostrzeżenia i liczba zredagowanych tokenów, zakładka/podgląd YAML z przyciskiem „Kopiuj YAML”, przyciski „Zapisz runbook” i „Anuluj”.
- [x] 4.3 Dodać testy komponentu / przepływu ekstrakcji i potwierdzenia w UI (`vitest`) oraz uruchomić `npm run typecheck` i `npm run build`.

## 5. Konfiguracja, dokumentacja i weryfikacja

- [x] 5.1 Dodać opcjonalne przekazywanie zmiennych `LLM_PROVIDER`, `LLM_BASE_URL`, `LLM_MODEL`, `LLM_API_KEY`, `LLM_TIMEOUT_MS` w `docker-compose.yml` (domyślnie `LLM_PROVIDER: ${LLM_PROVIDER:-none}` w `&backend-env`).
- [x] 5.2 Zaktualizować `docs/MOCKS.md` i `README.md` (sekcja „Użycie AI”) o instrukcję uruchomienia z `local` (Ollama) i `api` (OpenAI-compatible).
- [x] 5.3 Zweryfikować całość: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `npm run typecheck`, `npm test` w `ui/` oraz manualny test end-to-end z `LLM_PROVIDER=none` i z wybranym dostawcą OpenAI-compatible.
