# runbook-execution Specification

## Purpose

Wykonywanie runbooków AUTOMATIC dla alertów: sekwencja kroków, zatwierdzenia przez człowieka, obsługa porażek oraz dziennik tego, co i kto zrobił.

## Requirements

### Requirement: Uruchomienie raz na epizod
System SHALL automatycznie uruchomić każdy dopasowany runbook AUTOMATIC dokładnie raz na epizod alertu (przy otwarciu i przy ponownym otwarciu), a nie przy każdym zdeduplikowanym zdarzeniu.

#### Scenario: 500 zdarzeń, jedno uruchomienie
- **WHEN** 500 zdarzeń o tym samym fingerprincie tworzy jeden alert
- **THEN** runbook zostaje uruchomiony jeden raz

#### Scenario: Ponowne otwarcie
- **WHEN** alert zostaje ponownie otwarty w epizodzie 2
- **THEN** runbook zostaje uruchomiony ponownie, a uruchomienie z epizodu 1 pozostaje w historii

### Requirement: Wykonanie sekwencyjne
Kroki SHALL być wykonywane po kolei; kolejny krok SHALL rozpocząć się dopiero po pomyślnym zakończeniu poprzedniego.

#### Scenario: Dwa kroki automatyczne
- **GIVEN** runbook z krokami `http_get` (health check) i `retry_workflow`
- **WHEN** health check kończy się sukcesem
- **THEN** system wykonuje `retry_workflow`

### Requirement: Krok manualny wstrzymuje sekwencję
Krok manualny lub notatka SHALL wstrzymać sekwencję do czasu, gdy człowiek oznaczy go w UI jako wykonany (opcjonalnie z notatką).

#### Scenario: Oznaczenie kroku jako wykonanego
- **GIVEN** sekwencja zatrzymana na kroku manualnym
- **WHEN** użytkownik klika „Zrobione” i wpisuje notatkę
- **THEN** krok jest zakończony z notatką, a system przechodzi do kolejnego kroku

### Requirement: Zatwierdzenie z UI
Krok wymagający zatwierdzenia SHALL czekać, aż użytkownik zatwierdzi go lub odrzuci w UI; zatwierdzone kroki SHALL być wykonywane, a odrzucone SHALL zatrzymać sekwencję.

#### Scenario: Zatwierdzenie eskalacji
- **GIVEN** krok `escalate` z `approval: required` oczekujący w kolejce zatwierdzeń
- **WHEN** użytkownik go zatwierdza
- **THEN** akcja zostaje wykonana, a w dzienniku zapisane jest, kto zatwierdził

#### Scenario: Odrzucenie
- **WHEN** użytkownik odrzuca krok oczekujący na zatwierdzenie
- **THEN** akcja nie zostaje wykonana, a sekwencja jest zatrzymana z adnotacją o odrzuceniu

### Requirement: Kolejka zatwierdzeń
System SHALL udostępniać w UI listę wszystkich kroków oczekujących na zatwierdzenie, z alertem, akcją, parametrami i poziomem ryzyka.

#### Scenario: Przegląd kolejki
- **WHEN** użytkownik otwiera kolejkę zatwierdzeń
- **THEN** widzi oczekujące kroki z podstawionymi parametrami (np. nazwą usługi do restartu)

### Requirement: Porażka kroku zatrzymuje sekwencję
Gdy krok automatyczny zakończy się błędem, system SHALL zatrzymać sekwencję i pozwolić użytkownikowi ponowić krok, pominąć go albo przejść do trybu ręcznego.

#### Scenario: Nieudany health check
- **WHEN** krok `http_get` zwraca błąd
- **THEN** sekwencja jest zatrzymana, krok ma status błędu z jego treścią, a w UI dostępne są opcje „Ponów”, „Pomiń” i „Tryb ręczny”

#### Scenario: Pominięcie kroku
- **WHEN** użytkownik wybiera „Pomiń” przy nieudanym kroku
- **THEN** krok ma status pominięty, a system przechodzi do kolejnego kroku

### Requirement: Brak podwójnego wykonania
Krok SHALL zostać wykonany co najwyżej raz na próbę, także przy restarcie systemu w trakcie wykonania; akcje HTTP SHALL otrzymywać klucz idempotencji pozwalający systemowi docelowemu odrzucić duplikat.

#### Scenario: Restart w trakcie runbooka
- **WHEN** system zostaje zrestartowany po wykonaniu kroku 1, a przed krokiem 2
- **THEN** po restarcie wykonanie wznawia się od kroku 2, a krok 1 nie jest powtarzany

### Requirement: Dziennik działań
System SHALL zapisywać w dzienniku każde zdarzenie wykonania: start i wynik kroku, zatwierdzenie, odrzucenie, oznaczenie jako wykonany, pominięcie i ponowienie, z autorem (użytkownik lub system), czasem, epizodem i wynikiem.

#### Scenario: Historia działań przy alercie
- **WHEN** użytkownik otwiera szczegóły alertu
- **THEN** widzi chronologiczny dziennik: kto, co, kiedy i z jakim wynikiem, z podziałem na epizody

### Requirement: Status kroków widoczny na żywo
Status każdego kroku (oczekuje, w toku, sukces, błąd, czeka na zatwierdzenie, czeka na człowieka, pominięty) SHALL być widoczny w UI i aktualizowany bez odświeżania, z wyraźnym rozróżnieniem kroków automatycznych i manualnych.

#### Scenario: Postęp runbooka
- **WHEN** system kończy krok automatyczny
- **THEN** UI w ciągu kilku sekund pokazuje sukces kroku i start kolejnego
