# alert-rules Specification

## Purpose

Pozwala zespołom definiować, które alerty ich dotyczą, kto ma je dostać oraz jak mają wyglądać, za pomocą reguł zmienianych w trakcie działania systemu.

## Requirements

### Requirement: Warunki na dowolnym polu
Reguła SHALL zawierać warunek na polach kanonicznych zdarzenia oraz na dowolnym polu oryginału dostępnym jako `raw.<ścieżka>`, z operatorami: równe, różne, w zbiorze, większe/mniejsze (także lub równe), zawiera, wyrażenie regularne, istnieje, wzorzec z wildcardem.

#### Scenario: Reguła Rates
- **GIVEN** reguła `schema glob "trade.rejected.*" AND raw.payload.desk == "rates" AND raw.payload.notional > 10000000`
- **WHEN** przychodzi zdarzenie trade.rejected.v2 z `desk = rates` i `notional = 25000000`
- **THEN** reguła jest dopasowana

#### Scenario: Pole nieistniejące
- **WHEN** warunek odwołuje się do pola, którego nie ma w zdarzeniu
- **THEN** warunek jest niespełniony (poza operatorem „istnieje”) i nie powoduje błędu

### Requirement: Łączenie warunków AND/OR
Warunki SHALL dać się łączyć w zagnieżdżone grupy AND, OR oraz negację.

#### Scenario: Grupa OR wewnątrz AND
- **GIVEN** reguła `environment == prod AND (raw.error.code == TIMEOUT OR raw.error.code == REJECTED)`
- **WHEN** przychodzi zdarzenie z `environment = prod` i `error.code = REJECTED`
- **THEN** reguła jest dopasowana

### Requirement: Routing reguły wskazuje odbiorców
Reguła SHALL wskazywać odbiorców (osoby i/lub zespoły); kanały dostarczenia SHALL wynikać z preferencji odbiorców, a nie z reguły.

#### Scenario: Reguła na zespół
- **GIVEN** reguła Settlements wskazująca `team:settlements`
- **WHEN** reguła zostaje dopasowana
- **THEN** powiadomienie trafia do każdego członka zespołu zgodnie z jego preferencjami kanałów

### Requirement: Wiele dopasowanych reguł
Gdy alert pasuje do kilku reguł, system SHALL powiadomić sumę ich odbiorców, a ta sama osoba SHALL dostać na danym kanale co najwyżej jedno powiadomienie o danym alercie w danym epizodzie.

#### Scenario: Osoba w dwóch regułach
- **GIVEN** Anna jest odbiorcą reguły zespołowej i reguły osobistej, które pasują do tego samego alertu
- **WHEN** alert zostaje otwarty
- **THEN** Anna dostaje jedno powiadomienie na każdy swój kanał

### Requirement: Nadpisanie severity przez regułę
Reguła MAY nadpisać severity alertu dla swoich odbiorców; nadpisane severity SHALL być używane przy wyborze kanałów i prezentacji powiadomień tej reguły.

#### Scenario: Podniesienie severity
- **GIVEN** mapping nadaje `warning`, a reguła Rates nadpisuje severity na `critical`
- **WHEN** reguła zostaje dopasowana
- **THEN** odbiorcy reguły dostają powiadomienie jak dla `critical`

### Requirement: Prezentacja per reguła
Reguła SHALL definiować prezentację alertu: szablon tytułu i treści z podstawianiem pól (`{{pole.zagnieżdżone}}`), listę wyświetlanych pól oraz kolor/wyróżnienie w UI.

#### Scenario: Szablon tytułu
- **GIVEN** szablon tytułu `Rejected {{raw.payload.currency}} {{raw.payload.notional}} — {{raw.payload.reason}}`
- **WHEN** reguła zostaje dopasowana dla zdarzenia z Appendix A
- **THEN** tytuł powiadomienia brzmi `Rejected EUR 25000000 — LIMIT_BREACH`

#### Scenario: Brakująca zmienna w szablonie
- **WHEN** szablon odwołuje się do nieistniejącego pola
- **THEN** w to miejsce wstawiany jest pusty tekst, a powiadomienie zostaje wysłane

#### Scenario: Szablon nie wykonuje kodu
- **WHEN** szablon zawiera próbę dostępu do plików, zmiennych środowiskowych lub wykonania kodu
- **THEN** system nie wykonuje tej operacji i odrzuca szablon przy zapisie

### Requirement: Zmiana reguł w runtime
System SHALL stosować nowe, zmienione i usunięte reguły do kolejnych alertów bez restartu, zarówno przy edycji przez API/UI, jak i przez zmianę pliku konfiguracyjnego.

#### Scenario: Edycja reguły w UI
- **WHEN** operator zmienia próg `notional` w regule Rates i zapisuje
- **THEN** kolejne alerty są oceniane wg nowego progu w ciągu kilku sekund, bez restartu

#### Scenario: Edycja pliku
- **WHEN** operator zmienia plik reguły w katalogu konfiguracji
- **THEN** zmiana jest widoczna w UI i stosowana bez restartu

### Requirement: Reguła w formie kreatora i YAML
System SHALL udostępniać regułę zarówno jako formularz z drzewem warunków, jak i jako tekst YAML, przy czym zmiana w jednej formie SHALL być odzwierciedlona w drugiej.

#### Scenario: Przełączenie widoku
- **WHEN** operator dodaje warunek w kreatorze i przełącza na YAML
- **THEN** YAML zawiera dodany warunek

#### Scenario: Niepoprawny YAML
- **WHEN** operator wpisuje YAML, który nie jest poprawną regułą
- **THEN** system pokazuje błąd ze wskazaniem miejsca i nie zapisuje reguły

### Requirement: Testowanie reguły na zdarzeniu
System SHALL pozwalać przetestować regułę na wklejonym lub wybranym niedawnym zdarzeniu i pokazać wynik każdego warunku z osobna oraz wyrenderowaną prezentację.

#### Scenario: Warunek niespełniony
- **WHEN** operator testuje regułę na zdarzeniu, w którym `notional = 5000000`
- **THEN** UI pokazuje, że warunek `notional > 10000000` jest niespełniony, a pozostałe spełnione

### Requirement: Podpowiedzi pól
Edytor reguł SHALL podpowiadać nazwy pól na podstawie ostatnich zdarzeń wybranego schematu.

#### Scenario: Podpowiedź pola zagnieżdżonego
- **WHEN** operator wybiera schemat `trade.rejected.v2` i zaczyna wpisywać nazwę pola
- **THEN** edytor podpowiada m.in. `raw.payload.notional` i `raw.payload.desk`
