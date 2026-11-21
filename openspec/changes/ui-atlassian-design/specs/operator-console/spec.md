## ADDED Requirements

### Requirement: Statusy czytelne bez rozróżniania kolorów
Każdy status prezentowany w UI (stan alertu, severity, dostarczenie, krok runbooka, ryzyko i zatwierdzenie, dostępność kanału, stan reguły, połączenie na żywo) SHALL być pokazany jako etykieta tekstowa po polsku wraz z ikoną; kolor SHALL być wyłącznie informacją dodatkową. Kolor czerwony SHALL oznaczać tylko: odrzucone, blokujące, porażkę lub przekroczony termin.

#### Scenario: Status dostarczenia bez koloru
- **GIVEN** dostarczenie ze statusem `dlq`
- **WHEN** operator ogląda zakładkę dostarczeń alertu w trybie skali szarości
- **THEN** status jest rozpoznawalny po etykiecie „DLQ” i ikonie

#### Scenario: Potwierdzony alert nie jest czerwony
- **WHEN** alert ma stan `acked`
- **THEN** jego status jest pokazany jako „potwierdzony” z ikoną i w kolorze innym niż czerwony

### Requirement: Nieznane wartości statusu
Wartość statusu spoza znanej listy (np. nowa wartość z backendu) SHALL być pokazana neutralnie, z surową wartością jako etykietą, bez błędu renderowania widoku.

#### Scenario: Nieznany status z backendu
- **WHEN** API zwraca status kroku spoza znanej listy
- **THEN** UI pokazuje neutralny status z surową wartością jako etykietą, a reszta widoku renderuje się normalnie

### Requirement: Kolory z prezentacji reguły
Kolory nadane przez autora reguły w jej prezentacji (`color`, `highlight`) SHALL być stosowane niezależnie od reguły czerwieni dla statusów.

#### Scenario: Kolor z reguły Settlements zostaje
- **WHEN** użytkownik otwiera alert dopasowany regułą z `color: red`
- **THEN** alert jest wyróżniony na czerwono zgodnie z regułą

### Requirement: Motyw jasny i ciemny
UI SHALL stosować jasny lub ciemny motyw zgodnie z ustawieniem systemu operacyjnego i SHALL zmieniać motyw bez przeładowania strony, gdy ustawienie systemu się zmieni. Wszystkie ekrany SHALL zachować czytelność (kontrast tekstu co najmniej WCAG AA) w obu motywach.

#### Scenario: Ciemny motyw systemu
- **GIVEN** system operacyjny ustawiony na tryb ciemny
- **WHEN** operator otwiera dowolny ekran
- **THEN** ekran renderuje się w ciemnym motywie, bez jasnych plam z zakodowanych na stałe kolorów

### Requirement: Stany list: pusto, ładowanie, błąd
Każda lista i tabela danych w UI SHALL mieć rozróżnialny stan ładowania, stan pusty z opisem i stan błędu z możliwością ponowienia pobrania. Stan ładowania SHALL nie przesuwać układu strony po załadowaniu danych bardziej niż zajmuje sama lista.

#### Scenario: Błąd pobierania kolejki zatwierdzeń
- **WHEN** `GET /approvals` kończy się błędem
- **THEN** UI pokazuje komunikat błędu z przyciskiem „Spróbuj ponownie”, który ponawia pobranie

#### Scenario: Pusta kolejka
- **WHEN** nie ma kroków oczekujących na zatwierdzenie
- **THEN** UI pokazuje stan pusty z opisem zamiast pustej tabeli

### Requirement: Obsługa klawiaturą feedu alertów i kolejki zatwierdzeń
Feed alertów i kolejka zatwierdzeń SHALL dać się obsłużyć wyłącznie klawiaturą: `j`/`k` (i strzałki) przesuwają bieżący wiersz z widocznym fokusem, `Enter` otwiera alert bieżącego wiersza, `x` zaznacza wiersz do akcji zbiorczej, `Escape` czyści zaznaczenie lub zamyka otwartą warstwę. Skróty jednoklawiszowe SHALL NOT działać, gdy fokus jest w polu tekstowym, liście wyboru lub oknie dialogowym.

#### Scenario: Skrót w polu tekstowym
- **WHEN** fokus jest w polu tekstowym i operator wpisuje literę `a`
- **THEN** litera trafia do pola, a żaden krok nie zostaje zatwierdzony

### Requirement: Skróty decyzji w kolejce zatwierdzeń
W kolejce zatwierdzeń `a` SHALL zatwierdzać bieżący krok, a `r` SHALL otwierać potwierdzenie odrzucenia bieżącego kroku.

#### Scenario: Zatwierdzenie z klawiatury
- **GIVEN** operator z wybraną tożsamością jest w kolejce zatwierdzeń
- **WHEN** naciska `j`, a następnie `a`
- **THEN** pierwszy krok w kolejce zostaje zatwierdzony tak samo jak przyciskiem „Zatwierdź”

#### Scenario: Odrzucenie wymaga potwierdzenia
- **WHEN** operator naciska `r` na bieżącym kroku
- **THEN** UI prosi o potwierdzenie odrzucenia, a komenda `reject` jest wysyłana dopiero po potwierdzeniu

### Requirement: Akcje zbiorcze w kolejce zatwierdzeń
Kolejka zatwierdzeń SHALL pozwalać zaznaczyć wiele kroków i zatwierdzić lub odrzucić je jedną akcją. Odrzucenie zbiorcze SHALL wymagać potwierdzenia podającego liczbę kroków. Wynik SHALL być raportowany per krok: kroki zakończone sukcesem znikają z kolejki, a kroki z błędem (np. `409` — ktoś był szybszy) pozostają z komunikatem przy wierszu.

#### Scenario: Zatwierdzenie trzech kroków
- **GIVEN** operator zaznaczył 3 kroki w kolejce
- **WHEN** wybiera „Zatwierdź zaznaczone”
- **THEN** dla każdego kroku wysyłana jest komenda `approve` z jego tożsamością, a UI pokazuje podsumowanie wyniku (ile zatwierdzono, ile się nie udało)

#### Scenario: Częściowa porażka
- **GIVEN** jeden z zaznaczonych kroków został w międzyczasie zatwierdzony w innej karcie
- **WHEN** operator zatwierdza zaznaczone
- **THEN** pozostałe kroki są zatwierdzone, a przy kroku z odpowiedzią `409` widać komunikat, że krok nie czeka już na zatwierdzenie

### Requirement: Powiadomienie o wyniku akcji
Po wysłaniu decyzji (zatwierdzenie, odrzucenie, ack, resolve, ponowne wysłanie dostarczenia, zapis konfiguracji) UI SHALL pokazać nieblokujące powiadomienie z wynikiem; powiadomienie o sukcesie SHALL znikać samo, a o błędzie SHALL zostać do zamknięcia przez operatora. Powiadomienia SHALL być ogłaszane czytnikom ekranu.

#### Scenario: Zatwierdzenie z kolejki
- **WHEN** operator zatwierdza krok i API odpowiada `202`
- **THEN** krok znika z kolejki i pojawia się powiadomienie „Zatwierdzono” z odnośnikiem do alertu

### Requirement: Paleta komend
UI SHALL udostępniać paletę komend otwieraną skrótem `⌘K` (macOS) lub `Ctrl+K` (pozostałe systemy) z każdego ekranu, pozwalającą przejść do dowolnego ekranu z menu i wyszukać alert z feedu po tytule. `Escape` SHALL zamykać paletę i przywracać fokus do poprzedniego elementu.

#### Scenario: Przejście do kolejki zatwierdzeń
- **WHEN** operator naciska `⌘K`, wpisuje „zatw” i zatwierdza `Enter`
- **THEN** UI przechodzi do kolejki zatwierdzeń
