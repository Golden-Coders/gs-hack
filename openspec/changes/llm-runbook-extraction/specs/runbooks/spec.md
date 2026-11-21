# runbooks Specification (Delta)

## ADDED Requirements

### Requirement: Opcjonalna ekstrakcja kroków z tekstu przez LLM (`RunbookAuthor`)
System SHALL udostępniać opcjonalną ekstrakcję szkicu runbooka `AUTOMATIC` z tekstu lub Markdownu przez interfejs `RunbookAuthor` konfigurowany zmienną `LLM_PROVIDER` (`none` | `local` | `api`), działającą wyłącznie na żądanie (poza ścieżką przetwarzania alertów). Domyślną wartością SHALL być `none`.

#### Scenario: Domyślny dostawca `none`
- **GIVEN** usługa startuje bez ustawionego `LLM_PROVIDER` lub z `LLM_PROVIDER=none`
- **WHEN** klient żąda wygenerowania szkicu runbooka przez `POST /runbooks/draft`
- **THEN** system odpowiada błędem `501 Not Implemented` z czytelnym komunikatem, że ekstrakcja LLM jest wyłączona, a wszystkie runbooki `MANUAL` działają bez zmian jako tekst

#### Scenario: Nieznana wartość `LLM_PROVIDER`
- **GIVEN** usługa startuje z nieobsługiwaną wartością `LLM_PROVIDER=unknown`
- **WHEN** aplikacja inicjalizuje `RunbookAuthor`
- **THEN** start kończy się jawnym błędem konfiguracji zamiast cichego przełączenia na inny tryb

#### Scenario: Ekstrakcja przez klienta OpenAI-compatible (`local` lub `api`)
- **GIVEN** skonfigurowano `LLM_PROVIDER=local` (np. Ollama) lub `LLM_PROVIDER=api` (z `LLM_API_KEY`) oraz rejestr zawiera akcje `http_get`, `retry_workflow`, `escalate`
- **WHEN** operator przesyła tekst runbooka `MANUAL` do `POST /runbooks/draft`
- **THEN** system wysyła zredagowany tekst do endpointu `/v1/chat/completions` ze schematem JSON ograniczającym pole `action` do akcji z rejestru i zwraca szkic `RunbookDef` (`kind: automatic`) ze sklasyfikowanymi krokami, bez automatycznego zapisywania runbooka

#### Scenario: Niedostępność lub błąd modelu LLM
- **GIVEN** skonfigurowano `LLM_PROVIDER=local` lub `api`, lecz endpoint LLM jest niedostępny, przekracza timeout lub zwraca niepoprawny JSON
- **WHEN** operator wywołuje `POST /runbooks/draft`
- **THEN** system zwraca czytelny błąd (`502 Bad Gateway` lub `504 Gateway Timeout`), nie zapisuje żadnego runbooka i nie wpływa na przetwarzanie alertów

### Requirement: Redakcja danych wrażliwych przed wysłaniem do LLM
Przed wysłaniem treści runbooka do dostawcy LLM system SHALL zredagować adresy IP, adresy e-mail, pełne adresy URL oraz wewnętrzne nazwy hostów do deterministycznych placeholderów, zachowując zmienne szablonowe `{{...}}`. System SHALL NOT wysyłać do LLM danych z alertów, zdarzeń ani sekretów środowiskowych.

#### Scenario: Redakcja URL, IP i e-maila przed wywołaniem modelu
- **GIVEN** treść runbooka zawiera `Check http://internal-pay.corp:8080/health on 10.20.30.40 and notify ops@bank.internal for {{raw.workflowId}}`
- **WHEN** system przygotowuje prompt dla LLM
- **THEN** do LLM trafia tekst z placeholderami zamiast URL, IP i e-maila, zmienna `{{raw.workflowId}}` pozostaje nienaruszona, a odpowiedź z `POST /runbooks/draft` zawiera raport liczby zredagowanych elementów

### Requirement: Whitelista akcji i wymuszone zatwierdzenia dla szkicu z LLM
Każdy szkic zwrócony przez LLM SHALL przejść walidację po stronie backendu oraz `Registry::classify`: krok wskazujący akcję spoza rejestru SHALL zdegradować do `automation: manual`, a krok wskazujący akcję o `risk: high` SHALL mieć wymuszone `approval: required` niezależnie od odpowiedzi modelu.

#### Scenario: Model halucynuje nieznaną akcję
- **GIVEN** LLM zwrócił krok z `action: drop_database`, którego nie ma w rejestrze
- **WHEN** backend waliduje i klasyfikuje szkic
- **THEN** krok otrzymuje `automation: manual` (nie wykona się automatycznie) oraz ostrzeżenie w odpowiedzi szkicu

#### Scenario: Model pomija approval dla akcji wysokiego ryzyka
- **GIVEN** akcja `restart_service` ma w rejestrze `risk: high`, a LLM zwrócił krok z `approval: none`
- **WHEN** backend klasyfikuje szkic
- **THEN** krok ma wymuszone `approval: required`

### Requirement: Zapis zaakceptowanego runbooka z API
System SHALL udostępniać endpoint `POST /runbooks`, który zapisuje zaakceptowany przez człowieka runbook `AUTOMATIC` do bazy (`source_file = NULL`), z walidacją identyfikatora, kroków i klasyfikacją wg rejestru; hot reload `runbooks` SHALL włączać tak zapisany runbook do aktywnej migawki bez nadpisywania go przez synchronizację plików.

#### Scenario: Zapis zaakceptowanego szkicu
- **GIVEN** operator zaakceptował wygenerowany szkic runbooka `payments-auto`
- **WHEN** UI wysyła `POST /runbooks` z definicją runbooka
- **THEN** runbook zostaje zapisany w tabeli `runbooks`, pojawia się na `GET /runbooks` i jest aktywny dla kolejnych alertów bez restartu usług
