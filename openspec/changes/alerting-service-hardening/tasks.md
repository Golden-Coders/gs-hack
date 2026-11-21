# Tasks

Kontynuacja zarchiwizowanego `alerting-service-mvp` (numery w nawiasach kwadratowych to ID zadań MVP, gdzie jest pełny opis i historia). Właściciele strumieni i zasady jak w `CLAUDE.md`. Odhaczać dopiero po spełnieniu weryfikacji.

## 1. Przygotowanie środowiska (wszyscy)

- [x] 1.1 Przebudować stack z aktualnego `main` (`docker compose up -d --build`) w uzgodnionym oknie; weryfikacja: `docker inspect -f '{{.Created}}'` dla `ingest`, `lifecycle`, `router`, `sender`, `runbooks`, `api` późniejszy niż merge PR #32, `scripts/e2e/first-flow.sh` i `appendix-a.sh` przechodzą z `API_TOKEN=dev-token`

## 2. Odporność i skalowanie (P1+P2)

- [x] 2.1 [19.F1 → 18.4] `scripts/e2e/resilience.sh`: ruch z `loadgen`, w trakcie `docker compose stop mailpit`, wyłączenie SMS przez `PUT /mock/channels/sms`, restart kolejno `ingest`, `lifecycle`, `router`, `sender`; weryfikacja: liczba alertów = liczba unikalnych fingerprintów, brak duplikatów kluczy idempotencji w `deliveries`, fallback i DLQ widoczne w API
- [x] 2.2 Wyjaśnić różnicę 3 alertów względem unikalnych fingerprintów z pomiaru 1 (14 928 vs 14 931, rozgrzewka 1 019 vs 1 022); weryfikacja: przyczyna opisana w `docs/LOADTEST.md` (błąd liczenia w `loadgen` albo utrata w potoku — wtedy poprawka z testem)
- [x] 2.3 [19.G1 → 18.5] `docker compose up -d --scale lifecycle=2 --scale sender=2` pod obciążeniem; weryfikacja: dedup poprawny (jak w 2.1), pomiar dopisany do `docs/LOADTEST.md`
- [ ] 2.4 (opcjonalne, po 2.3) `sender`: przetwarzanie współbieżne z commitem offsetów w kolejności + pula połączeń SMTP (design, ustalenie 4); weryfikacja: `cargo test -p sender` zielone, test idempotencji bez zmian, lag `sender` przy 500 msg/s nie rośnie

## 3. Pomiar końcowy (P1+P2)

- [x] 3.1 [19.H2 → 5.3] Pomiar końcowy w `docs/LOADTEST.md` na obrazach z `main` (ustalenie 1), z rozbiciem p50/p99 na kanały, także bez email; weryfikacja: wiersz „Pomiar końcowy” wypełniony (msg/s, p50, p99, max lag, konfiguracja, komenda)

## 4. Weryfikacja UI na prawdziwym backendzie (P4)

- [ ] 4.1 [14.3] Kolejka zatwierdzeń; weryfikacja: zatwierdzenie usuwa krok z kolejki i aktualizuje szczegóły alertu
- [ ] 4.2 [14.4] Podgląd mocków SMS/telefonu z przełącznikiem; weryfikacja: wyłączenie SMS pokazuje fallback w szczegółach alertu (łańcuch z PR #29)
- [ ] 4.3 [15.1] Edytor reguł; weryfikacja: osoba spoza zespołu tworzy regułę w kreatorze bez pisania YAML
- [ ] 4.4 [15.2] Mappingi i UNMAPPED; weryfikacja: nowy schemat → alert w feedzie w < 2 min
- [ ] 4.5 [15.3] Formularz akcji HTTP; weryfikacja: dodanie `flush_cdn` zmienia krok runbooka `cdn-stale` na automatyczny
- [ ] 4.6 [16.1] `README.md`; weryfikacja: osoba spoza strumienia uruchamia demo ze świeżego klona wg README

## 5. Integracja i demo (wszyscy)

- [ ] 5.1 [18.3] Runbooki end-to-end (`scripts/e2e/runbooks.sh` z PR #31): Settlements z zatwierdzeniem i eskalacją, payments MANUAL w emailu, pricing MANUAL z linkiem w SMS; weryfikacja: kroki i dziennik widoczne w UI
- [ ] 5.2 [19.H1 + 18.6] Kryteria demo na czystym klonie (`scripts/e2e/criteria.sh`): nowy schemat z UI w < 2 min, nowa akcja HTTP w < 1 min, zmiana reguły bez restartu, runbook z poleceniem shell niczego nie wykonuje; weryfikacja: każdy punkt przećwiczony, opis przebiegu w PR
- [ ] 5.3 [17.1] `make demo` na czystym środowisku; weryfikacja: komenda przechodzi po `docker compose down -v`
- [ ] 5.4 [17.2] Próby generalne wg `docs/DEMO.md`; weryfikacja: 2 przebiegi bez ingerencji, nagranie ≤ 5 min

## 6. Otwarte kwestie (P1+P2, do decyzji)

- [x] 6.1 D10: `rule_ids`, `teams` i severity po `severity_override` w `GET /alerts` (dziś puste/z mappingu; filtr `?team=` w `GET /alerts` ignorowany, w SSE działa); weryfikacja: decyzja zapisana w `design.md`, a przy zmianie zachowania delta spec + test HTTP
- [ ] 6.2 Drobne znaleziska testera do decyzji: wyścig grup `sender`/`sender-retry` na tym samym kluczu (podwójna wysyłka w teorii), severity eskalacji z kontekstu zdarzenia zamiast reguły, opóźnienie ≤ 2 s widoku reguł w `runbooks`, brak pozytywnego testu reguły `pricing-warnings` i kanału `ui` jako głównego; weryfikacja: każde naprawione z testem albo świadomie odrzucone z adnotacją tutaj
