# notification-delivery Specification

## Purpose

Niezawodne dostarczanie powiadomień przez kanały UI, email, SMS i telefon, z ponawianiem, przełączaniem na kanały zapasowe i widocznym statusem każdego dostarczenia.

## Requirements

### Requirement: Kanały dostarczenia
System SHALL dostarczać powiadomienia kanałami: UI (push w czasie rzeczywistym), email (przez SMTP), SMS i telefon; SMS i telefon SHALL być symulowane, a ich „wysłane” wiadomości widoczne w UI.

#### Scenario: Email do Mailpit
- **WHEN** zlecono dostarczenie emailem
- **THEN** wiadomość pojawia się w skrzynce mock SMTP z tytułem i treścią wg prezentacji reguły

#### Scenario: Symulowany SMS
- **WHEN** zlecono dostarczenie SMS-em
- **THEN** wiadomość pojawia się w widoku „wysłane SMS/telefony” w UI z numerem odbiorcy i treścią

### Requirement: Treść dopasowana do kanału
Powiadomienie SHALL mieć treść dopasowaną do kanału: email i UI zawierają pełną treść i dołączone runbooki MANUAL, SMS zawiera skróconą treść i link do alertu w UI.

#### Scenario: SMS z linkiem
- **GIVEN** alert z dopasowanym runbookiem MANUAL
- **WHEN** wysyłany jest SMS
- **THEN** SMS zawiera tytuł alertu i link do jego szczegółów w UI, bez pełnej treści runbooka

### Requirement: Ponawianie przy błędzie technicznym
Przy błędzie technicznym kanału system SHALL ponawiać dostarczenie z rosnącymi odstępami; brak reakcji odbiorcy (np. nieodebrany telefon) SHALL NOT być traktowany jako błąd dostarczenia.

#### Scenario: Chwilowa niedostępność SMTP
- **GIVEN** serwer SMTP jest niedostępny przez 10 sekund
- **WHEN** zlecono dostarczenie emailem
- **THEN** system ponawia próbę i dostarcza email po przywróceniu SMTP

### Requirement: Fallback na kanał zapasowy
Po wyczerpaniu prób na danym kanale (lub przy błędzie trwałym) system SHALL przejść do następnego kanału z łańcucha odbiorcy, kończąc na UI.

#### Scenario: SMS niedostępny
- **GIVEN** łańcuch SMS → email → UI i wyłączony kanał SMS
- **WHEN** zlecono dostarczenie
- **THEN** po nieudanych próbach SMS system dostarcza emailem, a status pokazuje użycie fallbacku

### Requirement: Brak utraty powiadomień i DLQ
System SHALL NOT tracić zleconych dostarczeń przy awarii kanału lub restarcie; gdy zawiodą wszystkie kanały z łańcucha, dostarczenie SHALL trafić do kolejki nieudanych dostarczeń widocznej w UI z możliwością ręcznego ponowienia.

#### Scenario: Wszystkie kanały zawiodły
- **WHEN** wszystkie kanały z łańcucha zwracają błędy po wyczerpaniu prób
- **THEN** dostarczenie widnieje w UI jako nieudane z ostatnim błędem i przyciskiem „wyślij ponownie”

#### Scenario: Restart podczas dostarczania
- **WHEN** proces dostarczania zostaje zrestartowany w trakcie
- **THEN** po restarcie niedokończone dostarczenia są kontynuowane

### Requirement: Brak zdublowanych powiadomień
System SHALL dostarczyć danej osobie na danym kanale co najwyżej jedno powiadomienie o danym epizodzie alertu, także przy ponownym przetworzeniu po restarcie.

#### Scenario: Ponowne przetworzenie zlecenia
- **GIVEN** email do Anny o epizodzie 1 alertu został dostarczony
- **WHEN** to samo zlecenie zostaje przetworzone ponownie po restarcie
- **THEN** Anna nie dostaje drugiego emaila

### Requirement: Status dostarczenia
Każde dostarczenie SHALL mieć status (oczekuje, ponawiane, dostarczone, fallback, nieudane) widoczny przy alercie w UI i aktualizowany na żywo, wraz z czasem dostarczenia.

#### Scenario: Podgląd statusów przy alercie
- **WHEN** użytkownik otwiera szczegóły alertu
- **THEN** widzi listę dostarczeń (osoba, kanał, status, liczba prób, czas)

### Requirement: Dodanie realnego dostawcy jako izolowana zmiana
Kanały SHALL być zaimplementowane za wspólnym interfejsem adaptera tak, by zamiana mocka na realnego dostawcę wymagała zmiany wyłącznie w jednym adapterze i konfiguracji.

#### Scenario: Zamiana mocka SMS
- **WHEN** zespół dodaje adapter realnego dostawcy SMS
- **THEN** reguły, routing i pozostałe kanały nie wymagają zmian
