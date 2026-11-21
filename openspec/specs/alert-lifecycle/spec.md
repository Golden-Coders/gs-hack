# alert-lifecycle Specification

## Purpose

Zamiana strumienia zdarzeń w alerty: deduplikacja po fingerprincie, stany alertu (otwarty, potwierdzony, rozwiązany, ponownie otwarty) i historia epizodów.

## Requirements

### Requirement: Otwarcie alertu
System SHALL otworzyć nowy alert w stanie OPEN, gdy przychodzi zdarzenie, dla którego fingerprintu nie istnieje żaden alert.

#### Scenario: Pierwsze zdarzenie
- **WHEN** przychodzi zdarzenie o fingerprincie F, dla którego nie ma alertu
- **THEN** powstaje alert w stanie OPEN z licznikiem 1 i rozpoczyna się jego przetwarzanie przez reguły i runbooki

### Requirement: Deduplikacja do czasu rozwiązania
System SHALL łączyć kolejne zdarzenia o tym samym fingerprincie z istniejącym alertem, dopóki alert nie jest RESOLVED, zwiększając licznik i aktualizując czas ostatniego wystąpienia, bez ponownego powiadamiania.

#### Scenario: 500 identycznych zdarzeń w minutę
- **WHEN** w ciągu minuty przychodzi 500 zdarzeń o tym samym fingerprincie
- **THEN** istnieje jeden alert z licznikiem 500, a odbiorcy dostali jedno powiadomienie na kanał

#### Scenario: Zdarzenie dla potwierdzonego alertu
- **GIVEN** alert w stanie ACKED
- **WHEN** przychodzi kolejne zdarzenie o tym samym fingerprincie
- **THEN** licznik rośnie, a alert pozostaje ACKED bez ponownego powiadomienia

### Requirement: Potwierdzenie alertu
System SHALL pozwalać użytkownikowi przenieść alert ze stanu OPEN do ACKED, zapisując kto i kiedy to zrobił.

#### Scenario: Ack z UI
- **WHEN** użytkownik klika „Ack” przy otwartym alercie
- **THEN** alert przechodzi w stan ACKED z zapisanym użytkownikiem i czasem

### Requirement: Rozwiązanie alertu
System SHALL przenosić alert do stanu RESOLVED, gdy użytkownik go rozwiąże albo gdy przyjdzie zdarzenie oznaczone jako `resolved` o tym samym fingerprincie.

#### Scenario: Ręczne rozwiązanie
- **WHEN** użytkownik klika „Resolve”
- **THEN** alert przechodzi w RESOLVED z zapisanym użytkownikiem i czasem

#### Scenario: Automatyczne rozwiązanie
- **GIVEN** otwarty alert HighErrorRate dla payments-api
- **WHEN** przychodzi zdarzenie z tym samym fingerprintem i `resolved = true`
- **THEN** alert przechodzi w RESOLVED z adnotacją, że rozwiązało go źródło

### Requirement: Ponowne otwarcie i epizody
System SHALL ponownie otworzyć ten sam alert (REOPENED, nowy epizod), gdy przyjdzie zdarzenie o fingerprincie alertu RESOLVED, zachowując historię wcześniejszych epizodów, i SHALL ponownie powiadomić odbiorców.

#### Scenario: Problem wraca
- **GIVEN** alert RESOLVED w epizodzie 1
- **WHEN** przychodzi zdarzenie o tym samym fingerprincie
- **THEN** alert jest REOPENED w epizodzie 2, historia epizodu 1 (licznik, działania, czasy) jest dostępna, a odbiorcy dostają powiadomienie

### Requirement: Publikacja zmian alertu
System SHALL publikować każde przejście stanu alertu (otwarcie, ack, rozwiązanie, ponowne otwarcie) oraz zmiany licznika tak, by dalsze etapy i UI otrzymywały je bez odpytywania.

#### Scenario: Live feed licznika
- **WHEN** licznik alertu rośnie w wyniku deduplikacji
- **THEN** UI wyświetla zaktualizowany licznik w ciągu kilku sekund bez odświeżania strony

### Requirement: Skalowanie bez utraty spójności
System SHALL zachowywać poprawną deduplikację przy wielu równoległych instancjach przetwarzania: zdarzenia o tym samym fingerprincie SHALL NOT tworzyć więcej niż jednego otwartego alertu.

#### Scenario: Dwie instancje przetwarzania
- **GIVEN** dwie działające instancje etapu cyklu życia
- **WHEN** przychodzi 100 zdarzeń o tym samym fingerprincie
- **THEN** istnieje dokładnie jeden alert z licznikiem 100

#### Scenario: Przełączenie instancji w trakcie
- **WHEN** jedna z instancji zostaje zatrzymana w trakcie napływu zdarzeń o tym samym fingerprincie
- **THEN** nadal istnieje dokładnie jeden alert; licznik może być zaniżony o zdarzenia z ostatniej chwili przed zatrzymaniem, ale żaden alert nie ginie
