# action-registry Specification

## Purpose

Rejestr akcji, które runbooki mogą wykonywać: akcje wbudowane i akcje HTTP definiowane konfiguracją, z poziomem ryzyka i zabezpieczeniami ograniczającymi, co i gdzie może zostać wykonane.

## Requirements

### Requirement: Akcje wbudowane
System SHALL udostępniać wbudowane akcje: `escalate` (powiadomienie innego zespołu), `notify` (powiadomienie wskazanych osób) i `http_get` (sprawdzenie adresu, np. health check).

#### Scenario: Eskalacja do zespołu
- **WHEN** wykonywana jest akcja `escalate` z parametrem `team: counterparty-support`
- **THEN** członkowie zespołu counterparty-support dostają powiadomienie o alercie zgodnie ze swoimi preferencjami

### Requirement: Akcje deklaratywne HTTP
Operator SHALL móc dodać akcję HTTP przez formularz w UI lub plik YAML, określając nazwę, metodę, URL, nagłówki i treść (z podstawianiem wartości z alertu i parametrów kroku), poziom ryzyka i timeout; akcja SHALL być dostępna bez restartu i bez zmian w kodzie.

#### Scenario: Dodanie akcji w UI
- **WHEN** operator dodaje w formularzu akcję `flush_cache` (`POST http://cache-admin/{{service}}/flush`, `risk: high`)
- **THEN** akcja pojawia się w rejestrze w ciągu kilku sekund i może być użyta w runbookach

### Requirement: Poziom ryzyka wymusza zatwierdzenie
Każda akcja SHALL mieć poziom ryzyka `low` lub `high`; akcja `high` SHALL zawsze wymagać zatwierdzenia przez człowieka, niezależnie od ustawień w runbooku.

#### Scenario: Runbook próbuje pominąć zatwierdzenie
- **GIVEN** akcja `restart_service` z `risk: high`
- **WHEN** runbook używa jej z `approval: none`
- **THEN** krok i tak wymaga zatwierdzenia w UI przed wykonaniem

### Requirement: Wykonywanie wyłącznie akcji z rejestru
System SHALL wykonywać wyłącznie akcje zarejestrowane w rejestrze; treść runbooka SHALL NOT móc spowodować wykonania poleceń powłoki, kodu ani wywołań spoza rejestru.

#### Scenario: Runbook z poleceniem powłoki
- **WHEN** runbook zawiera krok z `action: shell` i `command: "rm -rf /"`
- **THEN** nic nie zostaje wykonane, a krok jest manualny z oznaczeniem „brak automatyzacji”

### Requirement: Allowlista hostów
Akcje HTTP SHALL móc łączyć się wyłącznie z hostami z konfigurowalnej allowlisty i SHALL NOT podążać za przekierowaniami na hosty spoza niej.

#### Scenario: Host spoza allowlisty
- **WHEN** akcja ma URL wskazujący host spoza allowlisty
- **THEN** wykonanie kończy się błędem „host niedozwolony”, bez wysłania żądania

### Requirement: Limity wykonania
Akcje HTTP SHALL mieć timeout i limit rozmiaru odpowiedzi; przekroczenie SHALL kończyć krok błędem.

#### Scenario: Wolna usługa
- **GIVEN** akcja z timeoutem 5 s
- **WHEN** usługa docelowa nie odpowiada przez 10 s
- **THEN** krok kończy się błędem timeoutu po 5 s

### Requirement: Sekrety poza definicją akcji
Wartości sekretów używanych przez akcje (np. tokeny API) SHALL być pobierane ze zmiennych środowiskowych przez referencję i SHALL NOT być przechowywane w definicji akcji ani pokazywane w UI i dzienniku.

#### Scenario: Nagłówek z tokenem
- **GIVEN** akcja z nagłówkiem `Authorization: Bearer ${TICKET_API_TOKEN}`
- **WHEN** operator ogląda definicję akcji lub wpis dziennika jej wykonania
- **THEN** widzi referencję `${TICKET_API_TOKEN}`, a nie wartość tokenu

### Requirement: Symulowane systemy docelowe
System SHALL zawierać symulowane usługi docelowe dla akcji `retry_workflow`, `restart_service` i `open_ticket`, które rejestrują otrzymane wywołania i pokazują je na potrzeby demo.

#### Scenario: Retry workflow
- **WHEN** wykonywana jest akcja `retry_workflow` dla WF-20261023-00042
- **THEN** symulowana usługa rejestruje wywołanie z tym identyfikatorem i zwraca sukces
