# Spec Delta

## Purpose

Dołącza do alertów instrukcje rozwiązania: runbooki AUTOMATIC (kroki z akcjami z rejestru) i MANUAL (tekst lub Markdown), dopasowywane do alertów i wypełniane wartościami z wiadomości.

## ADDED Requirements

### Requirement: Dwa rodzaje runbooków
System SHALL obsługiwać runbooki AUTOMATIC, złożone z kroków (akcje z rejestru oraz kroki manualne/notatki), i MANUAL, będące wolnym tekstem lub Markdownem bez parsowania na kroki.

#### Scenario: Runbook Markdown jako MANUAL
- **WHEN** operator dodaje runbook Markdown „HighErrorRate on payments-api” jako MANUAL
- **THEN** runbook jest zapisany i wyświetlany jako sformatowany tekst

#### Scenario: Runbook YAML jako AUTOMATIC
- **WHEN** operator dodaje runbook YAML `settlement-timeout` z Appendix B
- **THEN** runbook jest zapisany jako lista kroków z przypisanymi akcjami i wymaganiami zatwierdzenia

### Requirement: Definicja runbooków w konfiguracji
Runbooki SHALL być definiowane w plikach konfiguracyjnych przeładowywanych bez restartu; niepoprawny runbook SHALL być odrzucony z czytelnym błędem bez wpływu na pozostałe.

#### Scenario: Niepoprawny runbook
- **WHEN** operator zapisuje plik runbooka z błędem składni
- **THEN** system raportuje błąd z nazwą pliku i miejscem, a wcześniej załadowane runbooki działają dalej

### Requirement: Dopasowanie runbooka do alertu
Runbook SHALL określać warunek `matches` w tym samym języku co reguły; skrót „przypięty do reguły” SHALL oznaczać dopasowanie do alertów tej reguły.

#### Scenario: Dopasowanie po polach
- **GIVEN** runbook z `matches: type == WORKFLOW_FAILED AND error.code == TIMEOUT`
- **WHEN** otwarty zostaje alert dla zdarzenia WORKFLOW_FAILED z kodem TIMEOUT
- **THEN** runbook jest dołączony do alertu

#### Scenario: Przypięcie do reguły
- **GIVEN** runbook przypięty do reguły Pricing
- **WHEN** alert zostaje dopasowany przez regułę Pricing
- **THEN** runbook jest dołączony do alertu

### Requirement: Wiele pasujących runbooków
Gdy do alertu pasuje kilka runbooków, system SHALL pokazać wszystkie, uporządkowane od najbardziej szczegółowego dopasowania.

#### Scenario: Ogólny i szczegółowy runbook
- **GIVEN** runbook ogólny dla wszystkich WORKFLOW_FAILED i szczegółowy dla kodu TIMEOUT
- **WHEN** otwarty zostaje alert WORKFLOW_FAILED z kodem TIMEOUT
- **THEN** UI pokazuje oba runbooki, szczegółowy jako pierwszy

### Requirement: Podstawianie wartości z alertu
Treść runbooka i parametry kroków SHALL być wypełniane wartościami z alertu (np. `{{workflowId}}`, nazwa usługi) w momencie dołączenia do alertu.

#### Scenario: Parametr akcji
- **GIVEN** krok `retry_workflow` z parametrem `workflowId: "{{raw.workflowId}}"`
- **WHEN** runbook zostaje dołączony do alertu dla WF-20261023-00042
- **THEN** krok ma parametr `workflowId = WF-20261023-00042`

### Requirement: Runbooki w powiadomieniach
Dopasowane runbooki SHALL być prezentowane na każdym kanale w odpowiedniej formie: pełna treść i kroki w UI i emailu, link do alertu w SMS.

#### Scenario: Email z runbookiem MANUAL
- **WHEN** wysyłany jest email o alercie z dopasowanym runbookiem MANUAL
- **THEN** email zawiera pełną treść runbooka z podstawionymi wartościami

### Requirement: Warunki w krokach jako tekst
Warunki kroków (np. „jeśli retry zawiedzie dwukrotnie”) SHALL być prezentowane jako instrukcja dla człowieka i SHALL NOT być ewaluowane automatycznie.

#### Scenario: Krok warunkowy z Appendix B
- **WHEN** runbook `settlement-timeout` zawiera krok `if: "retry fails twice"`
- **THEN** krok jest wyświetlany jako krok manualny z treścią warunku, czekający na decyzję człowieka

### Requirement: Nieznana akcja degraduje do kroku manualnego
Krok runbooka wskazujący akcję spoza rejestru SHALL zostać zapisany jako krok manualny z oznaczeniem „brak automatyzacji”, a system SHALL zliczać takie wystąpienia per akcja.

#### Scenario: Runbook od jury z nieznaną akcją
- **WHEN** operator dodaje runbook AUTOMATIC z krokiem `action: flush_cdn`, którego nie ma w rejestrze
- **THEN** runbook zostaje zapisany, krok jest manualny z oznaczeniem „brak automatyzacji”, a licznik dla `flush_cdn` rośnie

#### Scenario: Dodanie brakującej akcji
- **GIVEN** krok manualny z powodu brakującej akcji `flush_cdn`
- **WHEN** operator dodaje akcję `flush_cdn` do rejestru
- **THEN** przy kolejnym przeładowaniu krok staje się automatyczny, bez restartu
