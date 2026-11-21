# operator-console Specification

## Purpose

Interfejs webowy dla dyżurnych i autorów konfiguracji: feed alertów na żywo, praca z alertem i runbookami, kolejka zatwierdzeń oraz edytory reguł, mappingów i akcji.

## Requirements

### Requirement: Feed alertów na żywo
UI SHALL pokazywać listę alertów aktualizowaną w czasie rzeczywistym przez push (bez odświeżania strony), z filtrem „mój zespół / wszystkie” oraz stanem, severity (z uwzględnieniem nadpisania przez dopasowane reguły), `rule_ids`, `teams`, kolorem, licznikiem i czasem ostatniego wystąpienia; `GET /alerts?team=<id>` SHALL filtrować alerty po wyliczonych zespołach dopasowanych reguł.

#### Scenario: Nowy alert
- **WHEN** system otwiera nowy alert dla zespołu użytkownika
- **THEN** alert pojawia się w feedzie w ciągu kilku sekund bez odświeżania strony

#### Scenario: Filtr po zespole i nadpisanie severity w GET /alerts
- **GIVEN** otwarty alert pasujący do reguły `settlements-prod` (zespół `settlements`, `severity_override: critical`, `color: red`)
- **WHEN** klient wywołuje `GET /alerts?team=settlements`
- **THEN** odpowiedź zawiera ten alert z `rule_ids: ["settlements-prod"]`, `teams: ["settlements"]`, `severity: "critical"`, `color: "red"` i wyrenderowanym tytułem reguły, a alerty innych zespołów są pominięte

#### Scenario: Utrata połączenia
- **WHEN** połączenie push zostaje zerwane i przywrócone
- **THEN** UI uzupełnia zmiany, które nastąpiły w czasie przerwy

### Requirement: Uwierzytelnianie API i SSE
Wszystkie trasy API poza `/health` i preflightem CORS `OPTIONS` SHALL wymagać statycznego tokenu (`Authorization: Bearer <API_TOKEN>`, a dla strumienia SSE `/stream` także alternatywnie parametru query `?token=<API_TOKEN>`).

#### Scenario: Brak lub błędny token
- **WHEN** klient wywołuje `GET /alerts` lub `GET /stream` bez poprawnego tokenu
- **THEN** API odpowiada `401 Unauthorized` z `{"error": "unauthorized"}`

#### Scenario: Poprawny token dla SSE
- **WHEN** klient łączy się z `GET /stream?token=<API_TOKEN>`
- **THEN** API zwraca `200 OK` z `Content-Type: text/event-stream` i zdarzeniem `hello`

### Requirement: Wybór tożsamości
UI SHALL pozwalać wybrać, jako która osoba działa użytkownik; ten wybór SHALL określać filtr „mój zespół” i autora działań zapisywanych w dzienniku.

#### Scenario: Działanie jako Anna
- **GIVEN** użytkownik wybrał tożsamość „Anna”
- **WHEN** potwierdza alert
- **THEN** dziennik zapisuje Annę jako autora

### Requirement: Szczegóły alertu
Widok alertu SHALL pokazywać: pola i wygląd wg prezentacji dopasowanej reguły, licznik i epizody, dopasowane runbooki ze statusem kroków, przyciski ack/resolve, dostarczenia z ich statusami (z zakładką nieudanych dostarczeń) oraz dziennik działań.

#### Scenario: Alert Settlements
- **WHEN** użytkownik otwiera alert dopasowany regułą Settlements
- **THEN** krok workflow i komunikat błędu są wyróżnione na czerwono zgodnie z regułą

### Requirement: Kolejka zatwierdzeń w UI
UI SHALL zawierać widok kroków oczekujących na zatwierdzenie z możliwością zatwierdzenia lub odrzucenia każdego z nich.

#### Scenario: Zatwierdzenie z kolejki
- **WHEN** użytkownik zatwierdza krok w kolejce
- **THEN** krok znika z kolejki, a jego status w szczegółach alertu się aktualizuje

### Requirement: Edytor reguł
UI SHALL zawierać edytor reguł z kreatorem drzewa warunków zsynchronizowanym z YAML, podpowiedziami pól, edycją odbiorców i prezentacji oraz testowaniem reguły na zdarzeniu.

#### Scenario: Nowa reguła od zera
- **WHEN** osoba nietechniczna tworzy w kreatorze regułę „severity = critical AND service = payments-api → Anna” i ją zapisuje
- **THEN** kolejny pasujący alert trafia do Anny, bez pisania YAML

### Requirement: Mappingi i zdarzenia UNMAPPED
UI SHALL pokazywać listę schematów bez mappingu z przykładowym payloadem i liczbą zdarzeń oraz pozwalać utworzyć lub edytować mapping (pola, severity, fingerprint, resolved) z podglądem wyniku na przykładowym zdarzeniu.

#### Scenario: Nowy schemat od jury w mniej niż 2 minuty
- **GIVEN** zdarzenia nieznanego schematu widoczne jako UNMAPPED
- **WHEN** operator tworzy mapping, wybierając pola z przykładowego payloadu, i zapisuje
- **THEN** zdarzenia zostają przetworzone i pojawiają się w feedzie jako alerty

### Requirement: Formularz akcji HTTP
UI SHALL zawierać formularz dodawania i edycji akcji deklaratywnych HTTP oraz listę akcji brakujących w rejestrze wraz z liczbą odwołań do nich w runbookach.

#### Scenario: Uzupełnienie brakującej akcji
- **GIVEN** lista brakujących akcji zawiera `flush_cdn` (użyta w 1 runbooku)
- **WHEN** operator klika „Dodaj” przy `flush_cdn`, wypełnia formularz i zapisuje
- **THEN** `flush_cdn` znika z listy brakujących, a krok w runbooku staje się automatyczny

### Requirement: Podgląd symulowanych kanałów
UI SHALL zawierać widok „wysłane SMS/telefony” z symulowanych kanałów oraz możliwość wyłączenia i włączenia symulowanego kanału na potrzeby pokazania awarii.

#### Scenario: Demo awarii kanału
- **WHEN** operator wyłącza symulowany kanał SMS w UI
- **THEN** kolejne dostarczenia SMS kończą się błędem, a w szczegółach alertu widać ponawianie i fallback

### Requirement: Konfiguracja tylko w plikach
Runbooki oraz osoby, zespoły i preferencje SHALL być w UI widoczne w trybie tylko do odczytu, z informacją, który plik konfiguracyjny należy edytować.

#### Scenario: Podgląd preferencji
- **WHEN** użytkownik otwiera widok osób
- **THEN** widzi osoby, zespoły i preferencje oraz ścieżkę pliku, w którym są zdefiniowane
