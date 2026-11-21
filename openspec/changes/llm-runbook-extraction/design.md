# Design: Ekstrakcja kroków runbooka przez LLM (OpenAI-compatible)

## Context

Projekt ma już szkielet interfejsu `RunbookAuthor` w `crates/runbooks/src/author.rs` (obecnie synchroniczny, z implementacją `NoAuthor` dla `LLM_PROVIDER=none`). Usługa `crates/runbooks` jest osobnym workerem konsumującym Kafkę i okresowo synchronizującym `config/` z bazą Postgres (`store::reload_once`), podczas gdy komunikację HTTP z UI obsługuje `crates/api`.

Kluczowe ograniczenia z dotychczasowej architektury:
1. **Brak LLM na ścieżce krytycznej alertu** (D12, D14): `ingest -> lifecycle -> router -> sender -> runbooks engine` nie może robić żadnych wywołań modelu. Ekstrakcja jest wyłącznie operacją on-demand inicjowaną przez operatora z API/UI.
2. **Bezpieczeństwo w środowisku finansowym** (D11, D12): do LLM trafia wyłącznie zredagowany tekst runbooka (nigdy payload alertów ani sekrety), model wybiera akcje wyłącznie z bieżącego rejestru, a backend niezależnie klasyfikuje wynik przez `Registry::classify` (wymuszając `approval: required` dla `risk: high` i degradację nieznanych akcji do `manual`).
3. **Współistnienie runbooków z plików i z bazy**: `store::sync_dir` usuwa z tabeli `runbooks` wyłącznie wiersze, których `source_file` zaczyna się od ścieżki `config/runbooks/` (`LEFT(source_file, ...) = prefix`). Wiersze z `source_file IS NULL` są już bezpiecznie ładowane przez `store::load`, klasyfikowane przez `Registry::classify` i rozgłaszane przez `NOTIFY` + `ui.events` — dokładnie tak samo jak akcje tworzone z UI (`crates/api/src/routes/actions.rs`).

## Goals / Non-Goals

**Goals:**
- Jeden lekki klient HTTP OpenAI-compatible (`/v1/chat/completions` z `response_format: json_schema`) obsługujący `LLM_PROVIDER=local` (domyślnie Ollama) oraz `LLM_PROVIDER=api` (OpenAI / Gemini OpenAI-compat / OpenRouter), bez dodawania ciężkich zewnętrznych SDK.
- Deterministyczna redakcja URL-i, adresów IP, e-maili i wewnętrznych hostów przed wysłaniem tekstu do modelu (z zachowaniem zmiennych `{{...}}`).
- Współdzielenie logiki `author`, `redact`, `registry` i `parse` między `crates/runbooks` a `crates/api` bez duplikowania kodu.
- Pełny przepływ w UI: wygenerowanie szkicu → podgląd kroków, ostrzeżeń i YAML → jawna akceptacja (zapis do bazy lub skopiowanie YAML).
- Zero regresji przy domyślnym `LLM_PROVIDER=none`.

**Non-Goals:**
- Automatyczne parsowanie plików `.md`/`.txt` w tle podczas hot-reloadu (złamanie zasady „zawsze za akceptacją człowieka”).
- Edytor drag-and-drop kroków runbooka (wystarczy podgląd + edycja podstawowych metadanych jak `id`/`title` + opcjonalne usunięcie niechcianego kroku lub skopiowanie YAML).
- Automatyczna aktualizacja istniejących runbooków z dziennika incydentu (osobny temat).
- Wysyłanie jakichkolwiek danych alertów do LLM.

## Decisions

### 1. Współdzielenie modułów między `runbooks` i `api` przez `crates/runbooks/src/lib.rs`
- **Decyzja:** dodać `crates/runbooks/src/lib.rs` eksportujący `pub mod author; pub mod parse; pub mod redact; pub mod registry;` (analogicznie do `crates/lifecycle` i `crates/router`, które już mają `lib.rs` i są używane jako zależności path-crate przez `api` i `runbooks`).
- **Uzasadnienie:** `crates/api` obsługuje `POST /runbooks/draft` i `POST /runbooks` — potrzebuje dokładnie tego samego `RunbookAuthor`, `redact` oraz `Registry::classify` co `crates/runbooks`. Unika to duplikacji logiki bezpieczeństwa.
- **Alternatywa odrzucona:** wywoływanie `runbooks` po HTTP z `api` — usługa `runbooks` nie wystawia obecnie serwera HTTP; dodawanie go tylko dla jednego endpointu niepotrzebnie komplikuje topologię.

### 2. Jeden klient OpenAI-compatible z `reqwest` (już w workspace)
- **Decyzja:** zmienić `RunbookAuthor::draft` na `async` (przez `#[async_trait]` lub zwracany `Pin<Box<dyn Future>>` bez dodatkowych makr) z sygnaturą przyjmującą zredagowany tekst, metadane (opcjonalne `id`, `title`, `matches`, `rule_ids`) oraz listę dostępnych akcji `&[ActionDef]`, zwracającą `Result<DraftOutcome, AuthorError>`.
- **Konfiguracja zmiennymi środowiskowymi:**
  - `LLM_PROVIDER`: `none` (domyślne) | `local` | `api`
  - `LLM_BASE_URL`: domyślnie `http://localhost:11434/v1` dla `local`, `https://api.openai.com/v1` dla `api`
  - `LLM_MODEL`: domyślnie `qwen2.5:7b` dla `local`, `gpt-4o-mini` dla `api`
  - `LLM_API_KEY`: opcjonalny dla `local`, wymagany dla `api` (błąd startu, gdy brak przy `LLM_PROVIDER=api`)
  - `LLM_TIMEOUT_MS`: domyślnie `30000` (30 s, bo lokalne modele bywają wolniejsze na cold-start)
- **Protokół:** `POST {LLM_BASE_URL}/chat/completions` z `temperature: 0`, `response_format: { "type": "json_schema", "json_schema": { "name": "runbook_draft", "strict": true, "schema": ... } }`.
- **Fallback dla modeli bez strict `json_schema`:** jeśli dostawca zwróci błąd 400 na `json_schema` albo zwróci JSON w bloku markdown ```` ```json ... ``` ````, klient parsuje oczyszczony JSON i waliduje go rygorystycznie po stronie Rust (`serde` + `Registry::classify`).

### 3. Redakcja danych wrażliwych (`crates/runbooks/src/redact.rs`)
- **Decyzja:** deterministyczny redaktor oparty na `regex` (już w `crates/runbooks/Cargo.toml`):
  1. Najpierw chroni tokeny szablonowe `{{ ... }}` (tymczasowo wyklucza z dopasowań, aby `{{raw.service}}` albo `{{alert.url}}` nie zostały zniszczone).
  2. Redaguje w ustalonej kolejności:
     - URL-e (`https?://[^\s)"'<>]+`) → `[REDACTED_URL_n]`
     - adresy e-mail → `[REDACTED_EMAIL_n]`
     - adresy IPv4 (z opcjonalnym portem) → `[REDACTED_IP_n]`
     - nazwy hostów z domenami wewnętrznymi (`*.internal`, `*.corp`, `*.local`, `*.svc`, `*.cluster.local` z opcjonalnym portem) → `[REDACTED_HOST_n]`
  3. Zwraca `RedactedText { text: String, count: usize, kinds: BTreeMap<String, usize> }`.
- **Uzasadnienie:** redakcja jest jednostronna przed wysłaniem do LLM — nie przywracamy na ślepo wrażliwych hostów w parametrach akcji (akcje deklaratywne HTTP i tak mają własne szablony URL w rejestrze i allowlistę hostów). Operator widzi w podglądzie licznik zredagowanych elementów i może w razie potrzeby uzupełnić parametr przed zapisem.

### 4. Kontrakt JSON Schema dla LLM i klasyfikacja wyniku
- **Schemat odpowiedzi modelu:**
  ```json
  {
    "title": "string",
    "steps": [
      {
        "id": "s1",
        "type": "check | action | manual | note",
        "text": "Opis kroku dla operatora",
        "action": "<enum z rejestru akcji> | null",
        "params": { "...": "..." },
        "approval": "none | required",
        "condition": "opcjonalny warunek tekstowy | null"
      }
    ]
  }
  ```
- **Prompt systemowy** zawiera listę akcji z rejestru wraz z ich `description`, `risk` i `params_schema`, oraz twarde instrukcje:
  - używaj **wyłącznie** akcji z podanej listy; jeśli krok nie pasuje do żadnej akcji, ustaw `type: "manual"` i `action: null`;
  - dla warunków w tekście („jeśli X zawiedzie...”) wpisuj je w pole `condition` jako tekst i ustawiaj `type: "manual"`;
  - zachowuj zmienne `{{...}}` z tekstu wejściowego.
- **Walidacja backendu po odpowiedzi LLM:**
  - normalizacja brakujących/zduplikowanych `step.id` (`s1`, `s2`, ...),
  - budowa `RunbookDef { kind: RunbookKind::Automatic, ... }`,
  - uruchomienie `Registry::classify(&def)` — gwarantuje, że nieznana akcja (nawet jeśli model zignorował enum) dostanie `automation: Manual`, a każda akcja `risk: High` dostanie `approval: Required`,
  - wygenerowanie ostrzeżeń (`warnings: Vec<String>`) dla kroków zdegradowanych do `manual` lub z wymuszonym `approval`,
  - wyrenderowanie czytelnego podglądu YAML (`yaml: String`) zgodnego z `parse::parse_runbook`.

### 5. Nowe endpointy REST w `crates/api/src/routes/runbooks.rs`
- `GET /runbooks/author/status` → `{ "enabled": bool, "provider": "none|local|api", "model": string | null }`
- `POST /runbooks/draft`:
  - Request: `{ "runbook_id"?: string, "title"?: string, "text"?: string, "matches"?: Condition, "rule_ids"?: string[] }` (wymagane albo `runbook_id` wskazujące istniejący runbook `MANUAL`, albo niepuste `text`).
  - Response `200 OK`: `{ "draft": RunbookDef, "yaml": string, "redacted_count": number, "redacted_kinds": Record<string, number>, "warnings": string[] }`.
  - Błędy: `501` gdy `LLM_PROVIDER=none`, `400` gdy brak tekstu / niepoprawne wejście, `404` gdy `runbook_id` nie istnieje, `502`/`504` gdy LLM zawiedzie.
- `POST /runbooks`:
  - Request: `RunbookDef` (zaakceptowany przez człowieka szkic; `kind` = `automatic`).
  - Walidacja: niepuste i poprawne `id` (`[a-zA-Z0-9_-]+`), niepusta lista `steps`, unikalne `step.id`, brak kolizji z runbookiem pochodzącym z pliku (`409 Conflict` z komunikatem, jeśli istnieje runbook z `source_file IS NOT NULL` o tym samym `id`).
  - Zapis do `runbooks (id, definition, resolved, source_file)` z `source_file = NULL` + `resolved` z `Registry::classify`, zwraca `201 Created` (lub `200 OK` przy aktualizacji runbooka z bazy, jeśli `overwrite` lub osobny `PUT` — dla prostoty `POST /runbooks` zwraca `409` przy istniejącym `id`, a operator może zmienić `id` w podglądzie).

## Risks / Trade-offs

- **Różnice w obsłudze `strict: true` między Ollama a OpenAI:** niektóre mniejsze modele lokalne w Ollama gorzej radzą sobie ze złożonym `additionalProperties` w `params` (obiekt dowolnych parametrów). **Mitygacja:** w schemacie JSON `params` zdefiniować jako obiekt (`{"type": "object"}`) bez `strict: true` albo z prostą listą par `[{ "key": "...", "value": "..." }]` konwertowaną w Rust do `Map<String, Value>` — podejście z tablicą `params_kv` działa niezawodnie w `strict: true` zarówno w OpenAI, jak i w Ollama.
- **Czas odpowiedzi lokalnego LLM (5–20 s):** może sprawiać wrażenie zawieszonego UI. **Mitygacja:** wyraźny stan ładowania w UI + konfigurowalny `LLM_TIMEOUT_MS` (domyślnie 30 s) + zero wpływu na inne żądania (asynchroniczne wywołanie).
- **Wrażliwość na fałszywe pozytywy w redakcji:** zbyt agresywny regex na nazwy hostów mógłby zredagować zwykłe nazwy usług (np. `payments-api`). **Mitygacja:** redagować tylko pełne URL-e, IPv4, e-maile oraz nazwy z jawnymi sufiksami domenowymi (`.internal`, `.corp`, `.local`, `.svc`, `.cluster.local`, `.lan`), nigdy pojedynczych nazw usług bez kropki.

## Open Questions

- Brak blokujących pytań — wszystkie wybory są zgodne z D12 i istniejącą architekturą bazy/API. Przy realizacji można zweryfikować na konkretnym lokalnym modelu (np. `qwen2.5:7b` w Ollama), czy `params` jako lista `{key, value}` daje lepszą stabilność niż wolny obiekt.
