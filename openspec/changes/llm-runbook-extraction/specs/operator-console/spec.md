# operator-console Specification (Delta)

## MODIFIED Requirements

### Requirement: Konfiguracja tylko w plikach
Osoby, zespoły i preferencje oraz runbooki zdefiniowane w plikach konfiguracyjnych SHALL być w UI widoczne w trybie tylko do odczytu z informacją o pliku źródłowym. Ekran Runbooki SHALL dodatkowo umożliwiać wygenerowanie szkicu runbooka `AUTOMATIC` z tekstu lub runbooka `MANUAL` za pomocą LLM, jego przegląd i jawne zapisanie przez operatora.

#### Scenario: Podgląd preferencji
- **WHEN** użytkownik otwiera widok osób
- **THEN** widzi osoby, zespoły i preferencje oraz ścieżkę pliku, w którym są zdefiniowane

#### Scenario: Runbook z pliku pozostaje tylko do odczytu
- **WHEN** użytkownik otwiera runbook załadowany z `config/runbooks/settlement-timeout.yaml`
- **THEN** widzi ścieżkę pliku źródłowego i kroki tylko do odczytu, bez bezpośredniej edycji pliku na dysku

## ADDED Requirements

### Requirement: Podgląd i akceptacja ekstrakcji runbooka przez LLM w UI
Ekran Runbooki SHALL udostępniać opcję wyodrębnienia kroków przez LLM z istniejącego runbooka `MANUAL` lub z wklejonego tekstu/Markdownu, z obowiązkowym ekranem podglądu kroków przed zapisem.

#### Scenario: Ekstrakcja z istniejącego runbooka MANUAL
- **GIVEN** `GET /runbooks/author/status` zwraca `enabled: true`
- **WHEN** operator klika „Wyodrębnij kroki (LLM)” przy runbooku `MANUAL`
- **THEN** UI wysyła żądanie `POST /runbooks/draft`, wyświetla podgląd proponowanych kroków (typ, akcja, parametry, `automation`, `approval`), informację o zredagowanych tokenach, podgląd YAML z możliwością skopiowania oraz przyciski „Zapisz runbook” i „Anuluj”

#### Scenario: Akceptacja i zapis szkicu
- **GIVEN** operator widzi podgląd szkicu wygenerowanego przez LLM
- **WHEN** operator klika „Zapisz runbook”
- **THEN** UI wysyła `POST /runbooks`, odświeża listę runbooków i wyświetla nowo zapisany runbook `AUTOMATIC`

#### Scenario: LLM wyłączony (`LLM_PROVIDER=none`)
- **GIVEN** `GET /runbooks/author/status` zwraca `enabled: false`
- **WHEN** operator otwiera ekran Runbooki
- **THEN** opcja ekstrakcji przez LLM jest nieaktywna z czytelną informacją, że wymaga ustawienia `LLM_PROVIDER=local` lub `api`
