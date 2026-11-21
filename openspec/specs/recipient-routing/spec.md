# recipient-routing Specification

## Purpose

Określa, kim są odbiorcy (osoby i zespoły) i na jakich kanałach każdy z nich chce dostawać alerty, w zależności od severity i pory dnia.

## Requirements

### Requirement: Osoby i zespoły
System SHALL przechowywać osoby (z danymi kontaktowymi per kanał) i zespoły (z listą członków), definiowane w pliku konfiguracyjnym przeładowywanym bez restartu.

#### Scenario: Dodanie osoby do zespołu
- **WHEN** operator dodaje osobę do zespołu w pliku konfiguracyjnym
- **THEN** kolejne alerty kierowane do tego zespołu trafiają także do tej osoby, bez restartu

### Requirement: Rozwinięcie zespołu na osoby
Gdy reguła wskazuje zespół, system SHALL wyznaczyć kanały osobno dla każdego członka zespołu.

#### Scenario: Zespół trzyosobowy
- **GIVEN** zespół settlements z trzema członkami o różnych preferencjach
- **WHEN** reguła wskazująca ten zespół zostaje dopasowana
- **THEN** każdy z trzech członków dostaje powiadomienie na swoich kanałach

### Requirement: Preferencje kanałów wg severity
Osoba SHALL mieć preferencje przypisujące severity do zestawu kanałów (np. critical → telefon + SMS, warning → email, info → tylko UI).

#### Scenario: Alert critical
- **GIVEN** osoba z preferencją `critical → phone, sms`
- **WHEN** dostaje alert o severity critical
- **THEN** system zleca dostarczenie przez telefon i SMS

### Requirement: Preferencje zależne od godzin
Preferencje SHALL móc zależeć od przedziału godzin w strefie czasowej osoby.

#### Scenario: Anna poza godzinami
- **GIVEN** Anna ma `critical → sms` w godzinach 08:00–20:00, a poza nimi `critical → email`
- **WHEN** alert critical przychodzi o 22:00 czasu Anny
- **THEN** system zleca dostarczenie emailem

### Requirement: Preferencje zespołu jako domyślne
Zespół SHALL móc definiować domyślne preferencje; preferencje osoby SHALL je nadpisywać, a gdy ani osoba, ani zespół ich nie określa, alert SHALL trafić co najmniej do UI.

#### Scenario: Osoba bez własnych preferencji
- **GIVEN** zespół z domyślnym `warning → email` i osoba bez własnych preferencji
- **WHEN** osoba dostaje alert warning
- **THEN** system zleca dostarczenie emailem

#### Scenario: Brak jakichkolwiek preferencji
- **WHEN** ani osoba, ani jej zespół nie mają preferencji dla danego severity
- **THEN** alert jest dostarczany do UI

### Requirement: Kanały zapasowe
Osoba SHALL móc zdefiniować uporządkowaną listę kanałów zapasowych używanych, gdy kanał podstawowy zawiedzie; UI SHALL być zawsze ostatnim kanałem zapasowym.

#### Scenario: Łańcuch fallbacku
- **GIVEN** osoba z kanałem podstawowym SMS i zapasowym email
- **WHEN** system zleca dostarczenie
- **THEN** zlecenie zawiera łańcuch SMS → email → UI
