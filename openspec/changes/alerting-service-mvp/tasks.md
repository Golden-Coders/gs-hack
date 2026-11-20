# Tasks

Podział na osoby, katalogi i punkty synchronizacji: `team.md`. Każda sekcja ma jednego właściciela (w nawiasie); wykonuje się ją przez `/openspec-apply-change alerting-service-mvp`, wskazując agentowi numer sekcji. Sekcja 1 jest wspólna i blokuje resztę; sekcje 2–17 idą równolegle w ramach strumieni; sekcja 18 to integracja.

## 1. Faza 0 — kontrakty i szkielet (wszyscy, ~1 h)

- [x] 1.1 Utworzyć Cargo workspace z pustymi crate'ami `contracts`, `condition`, `ingest`, `lifecycle`, `router`, `sender`, `runbooks`, `api`, `mock-targets`, `loadgen` i wspólnymi wersjami zależności w `[workspace.dependencies]`; weryfikacja: `cargo check --workspace` przechodzi
- [x] 1.2 W `contracts` zdefiniować typy z design D15 (`Envelope`, `CanonicalEvent`, `AlertChanged`, `AlertCommand`, `DeliveryRequest`, `RunbookCommand`, `UiEvent`, `RunbookDef`, `StepDef`, `ActionDef`, `Channel`, `Severity`), stałe nazw topików i funkcje kluczy idempotencji z D9; weryfikacja: test round-trip serde dla każdego typu
- [x] 1.3 W `condition` zamrozić publiczne API (`Condition` parse/compile, `evaluate`, `explain`, `render_template`) z implementacją-zaślepką; weryfikacja: crate kompiluje się i jest importowany przez `lifecycle`, `router`, `runbooks`
- [x] 1.4 Dodać `contracts/fixtures/`: trzy zdarzenia z Appendix A jako koperty oraz po jednej przykładowej wiadomości na każdy topik z D2; weryfikacja: test deserializuje wszystkie fixtures
- [x] 1.5 Napisać `contracts/openapi.yaml` (endpointy z sekcji 4, 7, 10, 11, 13 + `/stream` SSE) i wygenerować z niego mock serwer dla UI; weryfikacja: mock odpowiada na `GET /alerts`
- [x] 1.6 Szkielet `api` z modułami `p1.rs`, `p2.rs`, `p3.rs` i zamrożoną listą `merge(...)` w `main.rs`; weryfikacja: `GET /health` zwraca 200
- [x] 1.7 `docker-compose.yml` (redpanda, postgres, mailpit, binarki z jednego obrazu, ui) z utworzeniem topików z D2 (12 partycji) i Dockerfile z `cargo-chef`; weryfikacja: `docker compose up` startuje wszystkie kontenery, `rpk topic list` pokazuje topiki
- [x] 1.8 Konwencja migracji (`migrations/p1_*`, `p2_*`, `p3_*`, uruchamiane przy starcie) i katalog `config/` z podkatalogami właścicieli; weryfikacja: pusta migracja każdego strumienia wykonuje się przy starcie
- [x] 1.9 Szkielet `ui/` (Vite + React + TypeScript) z routingiem ekranów i klientem z OpenAPI; weryfikacja: `npm run dev` pokazuje pustą nawigację

## 2. Ingest (P1)

- [ ] 2.1 `POST /ingest` przyjmujący kopertę, nadający `event_id` i `received_at`, odpowiadający 202 po potwierdzeniu zapisu na `events.raw`; weryfikacja: test integracyjny z Redpandą + scenariusze „Poprawna koperta” i „Koperta bez wymaganego pola”
- [ ] 2.2 `POST /ingest/{schema}` budujący kopertę z `Content-Type`; weryfikacja: test dla JSON i YAML
- [ ] 2.3 Parsery formatów `json` i `yaml` za wspólnym interfejsem; nieobsługiwany format → 415; weryfikacja: testy jednostkowe parserów i kodów błędów
- [ ] 2.4 Konsument źródeł Kafki wg `config/p1/sources/*.yaml` (topic → schema, format), z przeładowaniem bez restartu; weryfikacja: dodanie pliku źródła powoduje konsumpcję nowego topiku bez restartu
- [ ] 2.5 Lista błędów ingestu (tabela + endpoint w `api/p1.rs`); weryfikacja: niepoprawny JSON z Kafki pojawia się w `GET /ingest/errors`

## 3. Normalizacja i UNMAPPED (P1)

- [ ] 3.1 Model i tabela `mappings` (pola-ścieżki, severity: ścieżka|stała|warunek, fingerprint, resolved) + seed `config/p1/mappings/` dla trzech schematów z Appendix A; weryfikacja: testy jednostkowe mapowania każdego fixture'a na oczekiwane zdarzenie kanoniczne
- [ ] 3.2 Rola normalizer: `events.raw` → `events.normalized` (klucz = fingerprint), normalizacja skali severity, domyślny fingerprint; weryfikacja: scenariusze z `event-normalization` jako testy
- [ ] 3.3 Zdarzenia bez mappingu → `events.unmapped` + tabela `unmapped_events` (liczba, przykład); weryfikacja: zdarzenie nieznanego schematu jest widoczne w `GET /unmapped`
- [ ] 3.4 Hot reload mappingów (pliki → upsert → `NOTIFY` → `ArcSwap`) i CRUD mappingów w `api/p1.rs` z podglądem wyniku na przykładowym payloadzie; weryfikacja: zmiana mappingu działa bez restartu
- [ ] 3.5 Ponowne przetworzenie UNMAPPED po zapisaniu mappingu; weryfikacja: scenariusz „Utworzenie mappingu z UI” jako test integracyjny

## 4. Cykl życia alertu (P1)

- [ ] 4.1 Tabele `alerts`, `alert_episodes` + rola dedup konsumująca `events.normalized` i `alerts.commands` z mapą stanu w pamięci; weryfikacja: test „500 identycznych zdarzeń” daje 1 alert z licznikiem 500 i jedno `alerts.changed {opened}`
- [ ] 4.2 Przejścia ack/resolve (komendy), auto-resolve ze zdarzenia `resolved`, reopen z nowym epizodem; weryfikacja: testy wszystkich scenariuszy `alert-lifecycle`
- [ ] 4.3 Okresowy zrzut liczników (~1 s) do Postgresa + `ui.events` z licznikiem; leniwe doczytanie stanu po rebalansie; weryfikacja: test z dwiema instancjami i zatrzymaniem jednej
- [ ] 4.4 Endpointy `api/p1.rs`: lista i szczegóły alertów, epizody, `POST /alerts/{id}/ack|resolve` publikujące `alerts.commands`; weryfikacja: testy HTTP

## 5. Test obciążenia (P1)

- [ ] 5.1 `loadgen`: ruch na HTTP i topik źródłowy, mieszanka schematów, sterowany odsetek duplikatów, zadane msg/s; weryfikacja: uruchomienie generuje zadaną liczbę zdarzeń
- [ ] 5.2 Zapytanie SQL p50/p99 (`delivered_at - received_at`) i skrypt zbierający lag konsumentów; weryfikacja: raport po przebiegu na pustym systemie
- [ ] 5.3 Pierwszy pomiar po pierwszym przepływie end-to-end (sekcja 18.1) i końcowy przed demo; wyniki w `docs/LOADTEST.md`; weryfikacja: plik zawiera msg/s, p50, p99, konfigurację i komendę do powtórzenia

## 6. Ewaluator warunków i szablony (P2, priorytet — blokuje P1 3.1 i P3 10.2)

- [ ] 6.1 Parsowanie i kompilacja warunków (`all/any/not`, operatory z D6, ścieżki kanoniczne i `raw.*`, prekompilowane regex/glob); weryfikacja: testy jednostkowe każdego operatora i braku pola
- [ ] 6.2 `explain` z wynikiem per liść; weryfikacja: test na regule Rates z niespełnionym progiem
- [ ] 6.3 Render szablonów w sandboxie (brak dostępu do plików/env, limit rozmiaru, brakująca zmienna → pusty tekst, odrzucenie niebezpiecznych szablonów przy zapisie); weryfikacja: testy scenariuszy „Szablon tytułu”, „Brakująca zmienna”, „Szablon nie wykonuje kodu”
- [ ] 6.4 Benchmark ewaluacji (np. 100 reguł × zdarzenie); weryfikacja: wynik zapisany w komentarzu do `docs/LOADTEST.md`

## 7. Reguły (P2)

- [ ] 7.1 Tabela `rules` (warunek, odbiorcy, nadpisanie severity, prezentacja, wersja) + seed `config/p2/rules/` z czterema regułami z zadania; hot reload; weryfikacja: zmiana pliku reguły działa bez restartu
- [ ] 7.2 CRUD reguł w `api/p2.rs` z walidacją YAML i błędami z lokalizacją; `POST /rules/test` (reguła + zdarzenie → `explain` + wyrenderowana prezentacja); weryfikacja: testy HTTP
- [ ] 7.3 Endpoint podpowiedzi pól z ostatnich zdarzeń danego schematu; weryfikacja: dla `trade.rejected.v2` zwraca `raw.payload.notional`

## 8. Routing (P2)

- [ ] 8.1 Tabele `people`, `teams`, `team_members`, `preferences` + seed `config/p2/people.yaml` (zespoły z zadania, Anna); hot reload; endpoint tylko do odczytu w `api/p2.rs`; weryfikacja: dodanie osoby w pliku działa bez restartu
- [ ] 8.2 Rola router: `alerts.changed {opened|reopened}` → dopasowane reguły → rozwinięcie zespołów → kanały wg severity i godzin (strefa osoby) → łańcuch fallbacku → `deliveries.requested` z treścią per kanał; weryfikacja: testy scenariuszy `recipient-routing` (w tym Anna o 22:00)
- [ ] 8.3 Deduplikacja odbiorców między regułami i dołączanie runbooków MANUAL (ewaluacja `matches` z tabeli `runbooks` tylko do odczytu; link w SMS, pełna treść w email/UI); weryfikacja: test „Osoba w dwóch regułach” i „SMS z linkiem”

## 9. Dostarczanie (P2)

- [ ] 9.1 Interfejs adaptera kanału i adaptery `ui`, `email` (SMTP → Mailpit), `sms_mock`, `phone_mock` (tabela `mock_outbox` + `ui.events`, przełącznik dostępności); weryfikacja: email widoczny w Mailpit, SMS w `GET /mock/outbox`
- [ ] 9.2 Rola sender z tabelą `deliveries` (klucz idempotencji z D9, statusy) i commitem offsetu po zapisie; weryfikacja: ponowne przetworzenie tego samego zlecenia nie wysyła drugi raz
- [ ] 9.3 Retry z backoffem przez `deliveries.retry`, fallback po łańcuchu, DLQ i `POST /deliveries/{id}/resend`; weryfikacja: test z wyłączonym SMS daje fallback na email; test z wyłączonymi wszystkimi kanałami daje wpis DLQ
- [ ] 9.4 Endpointy dostarczeń per alert i przełącznika symulowanych kanałów w `api/p2.rs`; weryfikacja: testy HTTP

## 10. Rejestr akcji i usługi docelowe (P3)

- [ ] 10.1 Rejestr z akcjami wbudowanymi `http_get`, `escalate`, `notify` (te dwie publikują `deliveries.requested`); weryfikacja: test `escalate` tworzy zlecenia dla członków zespołu
- [ ] 10.2 Akcje deklaratywne HTTP z tabeli `actions` + seed `config/p3/actions/`, szablony URL/body przez `condition`, referencje `${ENV}` do sekretów, hot reload; CRUD w `api/p3.rs`; weryfikacja: akcja dodana przez API jest dostępna bez restartu, sekret nie wycieka w odpowiedzi API
- [ ] 10.3 Zabezpieczenia: allowlista hostów, brak przekierowań poza allowlistę, timeout, limit odpowiedzi, `risk: high` ⇒ approval; weryfikacja: testy scenariuszy z `action-registry` (host spoza allowlisty, wolna usługa, pominięcie zatwierdzenia)
- [ ] 10.4 `mock-targets` z endpointami `retry_workflow`, `restart_service`, `open_ticket` i logiem wywołań (z obsługą `Idempotency-Key`); weryfikacja: wywołanie widoczne w `GET /calls`

## 11. Runbooki (P3)

- [ ] 11.1 Tabela `runbooks` + ładowanie `config/p3/runbooks/` (AUTOMATIC z YAML, MANUAL z `.md`/`.txt` z nagłówkiem `matches`), walidacja z błędami per plik, hot reload; seed: trzy runbooki z Appendix B; weryfikacja: niepoprawny plik nie psuje pozostałych
- [ ] 11.2 Kroki z nieznaną akcją → `manual` + licznik w `unhandled_actions`; kroki `if` → `note`; ponowna klasyfikacja po dodaniu akcji; weryfikacja: scenariusze „Runbook od jury z nieznaną akcją” i „Dodanie brakującej akcji”
- [ ] 11.3 Dopasowanie (`matches`, skrót do reguły), sortowanie wg szczegółowości, podstawianie wartości; endpoint runbooków per alert w `api/p3.rs`; weryfikacja: scenariusze „Wiele pasujących runbooków” i „Parametr akcji”
- [ ] 11.4 Interfejs `RunbookAuthor` z dostawcą `none` (LLM poza MVP); weryfikacja: crate kompiluje się z `LLM_PROVIDER=none`

## 12. Wykonanie runbooków (P3)

- [ ] 12.1 Konsument `alerts.changed` tworzący `runbook_runs` raz na epizod (klucz z D9); weryfikacja: 500 zdarzeń → 1 uruchomienie; reopen → nowe uruchomienie
- [ ] 12.2 Maszyna stanów kroków (sekwencja, `waiting_manual`, `waiting_approval`, `failed`) z zapisem przed wykonaniem i wznowieniem po restarcie; weryfikacja: test „Restart w trakcie runbooka”
- [ ] 12.3 Komendy `runbooks.commands` (approve, reject, done z notatką, retry, skip) i endpointy w `api/p3.rs`, w tym kolejka zatwierdzeń; weryfikacja: testy scenariuszy `runbook-execution`
- [ ] 12.4 Dziennik działań (`audit_log`) z autorem, epizodem i wynikiem + `ui.events` przy każdej zmianie kroku; weryfikacja: endpoint dziennika zwraca chronologiczną historię

## 13. UI — fundamenty (P4)

- [x] 13.1 Klient API z OpenAPI, przełącznik mock/prawdziwy backend, wybór tożsamości („działam jako”); weryfikacja: UI działa na mocku z sekcji 1.5
- [x] 13.2 Obsługa SSE `/stream` z ponownym połączeniem i dociągnięciem zmian z przerwy; weryfikacja: zerwanie połączenia i powrót uzupełnia feed
- [x] 13.3 Wspólne komponenty: badge severity/stanu, renderer prezentacji reguły (pola, kolor), renderer Markdown; weryfikacja: przegląd komponentów na danych z fixtures

## 14. UI — ekrany operacyjne (P4)

- [x] 14.1 Feed alertów na żywo z filtrem „mój zespół / wszystkie”; weryfikacja: nowy alert pojawia się bez odświeżania
- [ ] 14.2 Szczegóły alertu: prezentacja wg reguły, licznik, epizody, ack/resolve, runbooki ze statusem kroków i akcjami (zrobione, ponów, pomiń, tryb ręczny), dostarczenia z zakładką nieudanych i „wyślij ponownie”, dziennik; weryfikacja: scenariusz alertu Settlements z wyróżnieniem na czerwono
- [ ] 14.3 Kolejka zatwierdzeń; weryfikacja: zatwierdzenie usuwa krok z kolejki i aktualizuje szczegóły alertu
- [ ] 14.4 Podgląd symulowanych SMS/telefonów z przełącznikiem dostępności kanału; weryfikacja: wyłączenie SMS pokazuje fallback w szczegółach alertu

## 15. UI — ekrany konfiguracji (P4)

- [ ] 15.1 Edytor reguł: kreator drzewa warunków ↔ YAML, podpowiedzi pól, odbiorcy, prezentacja, „testuj na zdarzeniu”; weryfikacja: osoba spoza zespołu tworzy regułę w kreatorze bez pisania YAML
- [ ] 15.2 Mappingi i UNMAPPED: lista, przykładowy payload, tworzenie mappingu przez wybór pól, podgląd wyniku; weryfikacja: nowy schemat → alert w feedzie w < 2 min
- [ ] 15.3 Formularz akcji HTTP i lista brakujących akcji z „Dodaj”; weryfikacja: dodanie `flush_cdn` zmienia krok runbooka na automatyczny
- [ ] 15.4 Widoki tylko do odczytu: runbooki oraz osoby/zespoły/preferencje ze ścieżką pliku; weryfikacja: widoki pokazują dane z seedu

## 16. Dokumentacja (P4, z wkładem właścicieli)

- [ ] 16.1 `README.md`: uruchomienie w ≤ 2 komendach, architektura (diagram z design), jak dodać źródło, schemat, regułę, kanał, akcję, runbook; weryfikacja: osoba spoza strumienia uruchamia demo ze świeżego klona wg README
- [ ] 16.2 `docs/MOCKS.md` (co jest symulowane i jak podmienić na prawdziwe) oraz sekcja o bezpieczeństwie i użyciu AI w zgłoszeniu; weryfikacja: każdy mock z compose jest opisany

## 17. Dane i skrypt demo (P4, z wkładem właścicieli)

- [ ] 17.1 `make demo`: seed + generator przykładowych zdarzeń dla scenariusza (3 schematy, ×500 duplikatów, runbook z zatwierdzeniem, awaria kanału, nieznany schemat, runbook z nieznaną akcją); weryfikacja: komenda przechodzi na czystym środowisku
- [ ] 17.2 Scenariusz nagrania wideo (≤ 5 min) z kolejnością kroków i oczekiwanymi efektami; weryfikacja: próba generalna przechodzi 2× bez ingerencji

## 18. Integracja end-to-end (wszyscy)

- [ ] 18.1 Pierwszy przepływ: zdarzenie WORKFLOW_FAILED przez HTTP → alert → reguła Settlements → SMS (mock) i UI; weryfikacja: dostarczenie widoczne w UI z poprawnym `received_at`/`delivered_at`
- [ ] 18.2 Wszystkie trzy zdarzenia z Appendix A dają alerty i dostarczenia zgodnie z czterema regułami z zadania; weryfikacja: test integracyjny w compose
- [ ] 18.3 Runbooki end-to-end: Settlements (auto-retry + eskalacja z zatwierdzeniem), payments (MANUAL w emailu), pricing (MANUAL z linkiem w SMS); weryfikacja: kroki i dziennik widoczne w UI
- [ ] 18.4 Odporność: wyłączenie Mailpit i symulowanego SMS, restart każdej binarki w trakcie ruchu; weryfikacja: brak utraconych alertów i zdublowanych powiadomień
- [ ] 18.5 Skalowanie: 2 instancje `lifecycle` i `sender` pod obciążeniem; weryfikacja: poprawny dedup i pomiar w `docs/LOADTEST.md`
- [ ] 18.6 Kryteria demo: nowy schemat w < 2 min z UI, nowa akcja HTTP w < 1 min, zmiana reguły bez restartu, runbook z poleceniem shell niczego nie wykonuje; weryfikacja: każdy punkt przećwiczony na czystym klonie

## Workflow follow-up

- Po demo: zarchiwizować change (`/openspec-archive-change alerting-service-mvp`), co utworzy główne specs w `openspec/specs/`.
- Odłożone tematy (eskalacja przy braku ack, „Zapisz jako runbook”, propozycje zmian do alertów, LLM, edytory runbooków i osób w UI) jako osobne change'y.
