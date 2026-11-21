# Proposal: Alerting Service MVP

## Why

Sygnały z systemów (błędy, przekroczenia progów, zdarzenia biznesowe) rozchodzą się po logach, dashboardach i skrzynkach: właściwe osoby dowiadują się za późno albo toną w szumie. Budujemy na hackathon (18 h, 4 osoby) serwis, który przyjmuje zdarzenia w dowolnym schemacie, kieruje je do właściwych ludzi na preferowanym kanale i pomaga je rozwiązać runbookami z (częściowo) zautomatyzowanymi akcjami. Jury przyniesie na demo nieznany schemat i nieznany runbook, więc kluczowe jest, by oba dało się obsłużyć konfiguracją, bez kodu.

## What Changes

Nowy system od zera (repo nie zawiera jeszcze kodu). Ustalenia z sesji projektowej:

**Ingest i normalizacja**
- Wejście to zawsze koperta JSON `{format, schema, payload}`; `format` = `json | yaml | ...` (parser per format), `schema` wskazuje mapping.
- `POST /ingest` przyjmuje gotową kopertę; `POST /ingest/{schema}` przyjmuje surowy payload i sam buduje kopertę; konsument brokera mapuje `topic -> schema` z konfiguracji.
- Mapping (per schema) definiuje: ekstrakcję pól (JSONPath), domyślne `severity` (ścieżka | stała | warunek), `fingerprint` (lista pól) i opcjonalny warunek `resolved`.
- Brak mappingu dla `schema` -> zdarzenie zapisane jako **UNMAPPED**, widoczne w UI; po dodaniu mappingu zdarzenia są przetwarzane ponownie. Nic nie ginie.

**Cykl życia alertu i deduplikacja**
- Stany: `OPEN -> ACKED -> RESOLVED -> REOPENED`.
- Zdarzenia z tym samym fingerprintem łączą się z alertem, dopóki nie jest RESOLVED (licznik, `last_seen`, bez ponownego powiadomienia).
- RESOLVED ustawia człowiek albo zdarzenie spełniające warunek `resolved` z mappingu.
- Kolejne zdarzenie po RESOLVED -> **reopen** tego samego alertu (nowy epizod, historia zachowana) i ponowne powiadomienie.

**Reguły**
- Warunki na polach kanonicznych oraz `raw.*` (oryginał); operatory porównań, `in`, `contains`, `regex`, `exists`, wildcard; drzewo AND/OR.
- Reguła wskazuje **KTO** (osoby, zespoły) i może nadpisać severity oraz prezentację (szablony tytułu/treści, pola, kolor).
- Edytor: kreator w UI zsynchronizowany z YAML, podpowiedzi pól z ostatnich zdarzeń, **„testuj na zdarzeniu”**. Zmiany działają w runtime.
- Ten sam silnik warunków obsługuje `matches` runbooków.

**Routing i dostarczanie**
- Osoby i zespoły w bazie; preferencje osoby (severity + godziny -> kanały), preferencje zespołu jako domyślne. Bez harmonogramów on-call.
- Jednostka dostarczenia = `(alert, osoba, kanał)` ze statusem widocznym w UI.
- Kanały: UI (push), Email (mock SMTP), SMS i telefon (mock z podglądem); realny dostawca = jeden adapter.
- Porażka = błąd techniczny kanału. Retry z backoffem -> fallback na kanały zapasowe osoby -> UI -> DLQ z ręcznym „wyślij ponownie”.

**Runbooki i akcje**
- Runbook dopasowany przez `matches` (skrót: przypięcie do reguły); pasuje kilka -> pokazujemy wszystkie.
- **AUTOMATIC**: kroki = akcje z rejestru + kroki `manual/note`; w MVP tworzony w YAML (kreator w UI i tworzenie z opisu słownego przez LLM — na później). Wykonanie sekwencyjne, startuje od razu raz na epizod (open/reopen); krok `manual` czeka na „zrobione”, krok z approval czeka na zatwierdzenie w UI; porażka zatrzymuje wykonanie (ponów / pomiń / tryb ręczny). Warunki `if` są tekstem dla człowieka.
- **MANUAL**: wolny tekst / Markdown bez parsowania; w UI, pełny w emailu, link w SMS.
- Rejestr akcji: kodowe (wewnętrzne: `escalate`, `notify`) + **deklaratywne HTTP** (YAML lub formularz UI, hot reload). `risk: high` zawsze wymusza approval; allowlista hostów, timeouty; nic spoza rejestru się nie wykonuje.
- Dziennik działań: kto, co, kiedy, wynik, w ramach epizodu.

**UI**
- W UI: feed alertów na żywo, szczegóły alertu (pola wg reguły, licznik, epizody, runbooki ze statusem kroków, ack/resolve, dostarczenia + zakładka DLQ), kolejka zatwierdzeń, edytor reguł, mappingi + UNMAPPED, formularz akcji deklaratywnych, podgląd mocków SMS/telefon.
- Na start jako YAML z hot reloadem: runbooki, osoby/zespoły/preferencje.

**Poza zakresem MVP / otwarte**
- Otwarte: eskalacja / ponowne powiadomienie przy braku ack.
- Na później: „Zapisz jako runbook” z dziennika incydentu; propozycje zmian do alertów. **Ryzyko:** propozycje zmian to wymóg 2.4 zadania (nie opcja) — rozważyć minimalny formularz, jeśli zostanie czas.
- Na później: tworzenie runbooków z opisu słownego przez LLM (w MVP tylko interfejs z dostawcami `local` — domyślny — i `api`), edytory runbooków i osób/preferencji w UI, warunki `if` jako predykaty, grupowanie/throttling poza dedupem, on-call, zatwierdzanie odpowiedzią na alert.

## Podział pracy: 4 równoległe strumienie (agentic workflow)

Zasada: **kontrakty najpierw, potem własność rozłączna.** Każda osoba (z własnym agentem) pracuje wyłącznie w swoich katalogach i tabelach; strumienie łączą się tylko przez zamrożone kontrakty.

### Faza 0 (wspólna, ~1 h, przed rozejściem się)
Jeden PR `contracts/`, akceptowany przez wszystkich, zawierający:
- schematy danych: koperta, zdarzenie kanoniczne, Alert + epizod, Delivery, RunbookStep, ActionDefinition, AuditEntry, wynik ewaluacji warunku;
- nazwy i payloady kolejek/topiców między etapami (np. `events.normalized`, `alerts.changed`, `deliveries.requested`, `runbooks.triggered`);
- kontrakt API (OpenAPI) dla UI, z którego każdy strumień generuje mock;
- szkielet repo, `docker compose`, konwencję migracji (osobny katalog/prefiks per strumień), dane startowe z zadania (3 zdarzenia, reguły, runbooki).

Po Fazie 0 zmiana w `contracts/` = mały PR + ping do wszystkich; nikt nie edytuje cudzych katalogów.

### Strumienie

| # | Strumień | Własność (capabilities) | Wejście -> wyjście (kontrakt) |
|---|----------|-------------------------|-------------------------------|
| **P1** | Ingest i cykl życia | `event-ingestion`, `event-normalization`, `alert-lifecycle` | HTTP/broker -> `alerts.changed` (open/update/resolve/reopen); API mappingów i UNMAPPED |
| **P2** | Reguły, routing, dostarczanie | `alert-rules`, `recipient-routing`, `notification-delivery` | `alerts.changed` -> dopasowanie reguł -> `deliveries` ze statusami; biblioteka ewaluatora warunków dla P3; mocki kanałów |
| **P3** | Runbooki i akcje | `runbooks`, `action-registry`, `runbook-execution` | `alerts.changed` (+ ewaluator P2) -> dopasowane runbooki, wykonanie kroków, approvals, dziennik; API akcji i zatwierdzeń |
| **P4** | UI i doświadczenie operatora | `operator-console` | konsumuje API wszystkich + push; do czasu gotowości backendu pracuje na mockach z OpenAPI |

Dodatkowe odpowiedzialności przekrojowe (po jednej osobie, by uniknąć konfliktów):
- **P1:** skrypt testu obciążenia i pomiar msg/s, p50/p99 (P1 zna wejście i znaczniki czasu).
- **P2:** wspólny ewaluator warunków (reguły + `matches`) — P3 tylko go importuje.
- **P3:** interfejs dostawcy LLM (implementacja na później).
- **P4:** README, scenariusz demo, nagranie.

Punkty styku do pilnowania: ewaluator warunków (P2 -> P3), szablony `{{pole}}` (jedna implementacja, właściciel P2, używana przez P3), push do UI (emitują P1/P2/P3 na wspólny kanał zdarzeń z kontraktu, P4 tylko konsumuje).

## Capabilities

### New Capabilities
- `event-ingestion`: przyjmowanie zdarzeń przez HTTP (koperta i surowy payload) i broker, parsowanie wg `format`.
- `event-normalization`: mappingi per schema (pola, severity, fingerprint, resolved), obsługa UNMAPPED i ponownego przetwarzania.
- `alert-lifecycle`: deduplikacja po fingerprincie, stany alertu, epizody, reopen, auto-resolve.
- `alert-rules`: język warunków (pola kanoniczne + raw, AND/OR), routing KTO, nadpisanie severity i prezentacji, zmiany w runtime, test na zdarzeniu.
- `recipient-routing`: osoby, zespoły, preferencje kanałów wg severity i godzin, kanały zapasowe.
- `notification-delivery`: adaptery kanałów, retry, fallback, DLQ, statusy dostarczeń.
- `runbooks`: runbooki AUTOMATIC i MANUAL, dopasowanie przez `matches`, podstawianie wartości, prezentacja per kanał.
- `action-registry`: akcje kodowe i deklaratywne HTTP, poziom ryzyka, allowlista hostów.
- `runbook-execution`: sekwencyjne wykonanie kroków, approvals z UI, obsługa porażek, dziennik działań.
- `operator-console`: feed na żywo, szczegóły alertu, kolejka zatwierdzeń, edytory reguł, mappingów i akcji, podgląd mocków.

### Modified Capabilities
- (brak — brak istniejących specs)

## Impact

- Nowy kod w całości: backend w Rust (Cargo workspace: `contracts`, `condition`, `ingest`, `lifecycle`, `router`, `sender`, `runbooks`, `api`, `mock-targets`, `loadgen`), frontend TypeScript + React, konfiguracja YAML, `docker compose`, test obciążenia. Szczegóły: `design.md`.
- Infrastruktura: Kafka API jako źródło zdarzeń i szkielet między etapami (lokalnie Redpanda), Postgres jako jedyna baza, Mailpit jako mock SMTP. Bez Redisa.
- Kryteria oceny wymagają zmierzonej przepustowości i latencji (ingest -> dostarczenie, p50/p99), więc wolne operacje (w przyszłości LLM) muszą pozostać poza ścieżką krytyczną.
- Bezpieczeństwo: wykonywanie akcji tylko z rejestru, wymuszone approvals dla `risk: high`, allowlista hostów; gdy LLM wejdzie, trafia do niego wyłącznie zredagowany opis runbooka (do opisania w zgłoszeniu).
