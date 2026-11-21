# Podział pracy: 4 osoby, 4 agentów

Kontekst: `proposal.md` (co i dlaczego), `design.md` (jak), `specs/` (zachowanie), `tasks.md` (lista zadań). Ten plik mówi **kto co robi, czego nie dotyka i kiedy się synchronizujemy**.

| Strumień | Osoba | Język | Sekcje w `tasks.md` | Capabilities |
|---|---|---|---|---|
| **P1** Ingest i cykl życia | Krzysztof Jet | Rust | 2, 3, 4, 5 | `event-ingestion`, `event-normalization`, `alert-lifecycle` |
| **P2** Reguły, routing, dostarczanie | Krzysztof Wojt | Rust | 6, 7, 8, 9 | `alert-rules`, `recipient-routing`, `notification-delivery` |
| **P3** Runbooki i akcje | Kostek | Rust | 10, 11, 12 | `runbooks`, `action-registry`, `runbook-execution` |
| **P4** UI, dokumentacja, demo | Damian | TypeScript | 13, 14, 15, 16, 17 | `operator-console` |

**Aktualizacja 2026-11-21:** P1 i P2 mają zrobione rdzenie (sekcje 2, 4, 6–9), więc pozostała praca obu strumieni idzie **jednym pipeline'em P1+P2** (`tasks.md`, sekcja 19) z etapami A–H i bramkami testowymi. Pipeline może pisać w katalogach P1 i P2 oraz w `crates/api/src/stream.rs` (SSE, wcześniej „domyślnie P1”) i `scripts/e2e/`. Tabele, topiki i zasada single writer bez zmian — `lifecycle` nadal jedynym pisarzem stanu alertu.

Damian nie zna Rusta, więc dostaje strumień w całości po stronie TypeScript (UI + dokumentacja + demo) i nie dotyka crate'ów. Przydział P1–P3 między osoby znające Rusta jest umowny — zamieńcie się, jeśli ktoś woli inny obszar.

Sekcja 1 (Faza 0) robią wszyscy razem, sekcja 18 (integracja) wszyscy razem.

---

## Złote zasady (dla ludzi i agentów)

1. **Piszesz tylko w swoich katalogach i tabelach** (lista niżej). Cudzy kod tylko czytasz.
2. **`crates/contracts/` i `contracts/openapi.yaml` są zamrożone** po Fazie 0. Dozwolone zmiany: tylko addytywne (nowe pole jako `Option`, nowy endpoint). Mały PR + wiadomość na czacie zespołu. Zmiana łamiąca = zgoda wszystkich.
3. **Stan alertu zmienia tylko P1, stan runbooka tylko P3** — reszta wysyła komendy na `alerts.commands` / `runbooks.commands`.
4. **Zależności Rust dodajesz tylko w swoim `Cargo.toml`**; nowa wersja we `[workspace.dependencies]` = PR + ping.
5. **`api/src/main.rs` nie edytujesz** — routy dodajesz w swoim `api/src/pN.rs`.
6. Migracje: tylko `migrations/<wersja>_pN_*.sql` w swoim zakresie numerów (P1: 1001–1999, P2: 2001–2999, P3: 3001–3999), bez kluczy obcych do tabel innych strumieni.
7. Testujesz swój etap **sam**: publikujesz fixtures na swój topik wejściowy (`rpk topic produce`) i czytasz wyjściowy. Nie czekasz na innych.
8. Małe, częste merge'e do `main` (co 1–2 h), każdy z zielonym `cargo test -p <twój crate>`.

Krótka wersja do wklejenia agentowi:

> Pracujesz nad strumieniem PN w change `alerting-service-mvp`. Realizuj wyłącznie sekcje X, Y z `tasks.md`. Możesz edytować tylko: <katalogi z tabeli poniżej>. Nie zmieniaj `crates/contracts`, `contracts/openapi.yaml`, `api/src/main.rs` ani cudzych crate'ów — jeśli kontrakt nie wystarcza, zatrzymaj się i opisz potrzebną zmianę. Testuj przez fixtures z `crates/contracts/fixtures`.

---

## P1 — Ingest i cykl życia

**Cel:** każde zdarzenie bezpiecznie wchodzi do systemu, staje się zdarzeniem kanonicznym i alertem z deduplikacją. Plus pomiar wydajności.

| | |
|---|---|
| Pisze w | `crates/ingest`, `crates/lifecycle`, `crates/loadgen`, `api/src/p1.rs`, `migrations/1xxx_p1_*`, `config/p1/` (sources, mappings), `docs/LOADTEST.md` |
| Tabele | `ingest_errors`, `mappings`, `unmapped_events`, `alerts`, `alert_episodes` |
| Konsumuje | HTTP, zewnętrzne topiki Kafki, `events.raw`, `events.normalized`, `alerts.commands` |
| Produkuje | `events.raw`, `events.unmapped`, `events.normalized`, `alerts.changed`, `ui.events` |
| Zależy od | `condition` od P2 (warunki severity/resolved w mappingu) — do tego czasu zaślepka z Fazy 0 |
| Dostarcza innym | `alerts.changed` (P2, P3, api), endpointy alertów i mappingów (P4) |

Kolejność: 2.1 → 3.1–3.2 → 4.1 (**to odblokowuje pierwszy przepływ**) → 4.2 → 3.3–3.5 → 4.3–4.4 → 2.2–2.5 → 5.x.

## P2 — Reguły, routing, dostarczanie

**Cel:** dla każdego alertu ustalić kto i jakim kanałem go dostaje, wyrenderować treść i niezawodnie ją dostarczyć.

| | |
|---|---|
| Pisze w | `crates/condition`, `crates/router`, `crates/sender`, `api/src/p2.rs`, `migrations/2xxx_p2_*`, `config/p2/` (rules, people.yaml) |
| Tabele | `rules`, `people`, `teams`, `team_members`, `preferences`, `deliveries`, `mock_outbox` |
| Konsumuje | `alerts.changed`, `deliveries.requested`, `deliveries.retry`; czyta `runbooks` (tylko odczyt, runbooki MANUAL) |
| Produkuje | `deliveries.requested`, `deliveries.retry`, `deliveries.dlq`, `ui.events` |
| Zależy od | nic blokującego — testuje na fixtures `alerts.changed` |
| Dostarcza innym | **`condition` (P1, P3) — priorytet nr 1**, adaptery kanałów, endpointy reguł/dostarczeń (P4) |

Kolejność: **6.1–6.3 jak najszybciej (cel: ~H3)** → 9.1–9.2 (minimalny sender: UI + SMS mock) → 8.1–8.2 → 7.1 (**pierwszy przepływ**) → 9.3–9.4 → 7.2–7.3 → 8.3 → 6.4.

Uwaga: to najcięższy strumień. Plan awaryjny w sekcji „Bilans obciążenia”.

## P3 — Runbooki i akcje

**Cel:** alert przychodzi z instrukcją i automatycznymi krokami, wykonywanymi bezpiecznie, z zatwierdzeniami i dziennikiem.

| | |
|---|---|
| Pisze w | `crates/runbooks`, `crates/mock-targets`, `api/src/p3.rs`, `migrations/3xxx_p3_*`, `config/p3/` (runbooks, actions) |
| Tabele | `runbooks`, `actions`, `runbook_runs`, `step_runs`, `audit_log`, `unhandled_actions` |
| Konsumuje | `alerts.changed`, `runbooks.commands` |
| Produkuje | `deliveries.requested` (escalate/notify), `ui.events` |
| Zależy od | `condition` od P2 (`matches`, szablony parametrów) — do tego czasu zaślepka |
| Dostarcza innym | definicje runbooków (P2 czyta MANUAL), endpointy runbooków/akcji/zatwierdzeń (P4) |

Kolejność: 10.1, 10.4 → 11.1 → 12.1–12.2 → 10.2–10.3 → 11.2–11.3 (po `condition`) → 12.3–12.4 → 11.4.

## P4 — UI, dokumentacja, demo

**Cel:** wszystko, co widać na wideo, plus README i scenariusz demo.

| | |
|---|---|
| Pisze w | `ui/`, `README.md`, `docs/MOCKS.md`, `docs/DEMO.md`, `Makefile` (cel `demo`), `config/demo/` |
| Konsumuje | REST z `contracts/openapi.yaml`, SSE `/stream` |
| Zależy od | do ~H6 pracuje na mock serwerze z Fazy 0, potem przełącza się na prawdziwy backend |
| Dostarcza innym | zgłoszenia „brakuje mi pola/endpointu” jako PR do `openapi.yaml` |

Kolejność: 13.1–13.3 → 14.1–14.2 (najważniejsze dla wideo) → 15.2 → 15.1 → 14.3–14.4 → 15.3–15.4 → 16 → 17.

---

## Harmonogram i punkty synchronizacji

```
 H0 ──── H1 ─────────── H3 ─────────── H6 ──── H8 ─────────────── H14 ──── H16 ──── H18
 | Faza 0 |  rdzenie strumieni          |  SYNC  |  funkcje + UI         | integr. | demo |
 | wszyscy|                             |  e2e   |                       |         |      |
            P2: condition gotowe @H3 ──>|        |                       |         |      |
                                        | 18.1   | P1: loadtest #1       | 18.2-18.6        |
                                        | pierwszy                       |  P4: README,     |
                                        | przepływ                       |  make demo,      |
                                        |                                |  nagranie        |
```

| Kiedy | Punkt synchronizacji | Kto |
|---|---|---|
| H1 | Merge Fazy 0; od tej chwili kontrakty zamrożone | wszyscy |
| ~H3 | `condition` z prawdziwą implementacją na `main` (P1 i P3 usuwają zaślepkę) | P2 → P1, P3 |
| ~H6 | **Pierwszy przepływ** (18.1): HTTP → alert → reguła → SMS mock + UI | wszyscy, 30 min razem |
| ~H6 | P4 przełącza UI z mocka na prawdziwy backend | P4 |
| ~H8 | Pierwszy pomiar wydajności, wspólna decyzja co optymalizować | P1 + reszta |
| co 2 h | 10-min stand-up: co zmergowane, co blokuje, potrzebne zmiany kontraktu | wszyscy |
| ~H14 | Zamrożenie funkcji; od teraz tylko integracja i poprawki | wszyscy |
| H14–16 | Sekcja 18: wszyscy przechodzą scenariusze integracyjne | wszyscy |
| H16–18 | Próby demo na czystym klonie, nagranie | P4 prowadzi, reszta wspiera |

## Bilans obciążenia i plan awaryjny

| Strumień | Szacunek | Ryzyko |
|---|---|---|
| P1 | średnie–duże | dedup + rebalans; loadtest na końcu |
| P2 | **największe** | 4 sekcje, w tym biblioteka blokująca innych |
| P3 | średnie | maszyna stanów i idempotencja |
| P4 | duże | dużo ekranów, ale bez blokad (mocki) |

Jeśli P2 nie wyrabia: **P3 przejmuje sekcję 9 (sender)** po skończeniu 10–12 (sender ma własny crate i tabelę, więc przekazanie nie powoduje konfliktów). Jeśli P1 nie wyrabia: **sekcję 5 (loadtest)** przejmuje osoba, która pierwsza skończy swój zakres.

Kolejność cięcia zakresu, gdy brakuje czasu (od pierwszego do odcięcia): 7.3 podpowiedzi pól → 15.4 widoki tylko do odczytu → 4.3 test z dwiema instancjami (zostawić tylko opis) → 2.5 lista błędów ingestu → 13.2 dociąganie zmian po przerwie SSE. **Nie tniemy:** dedup, UNMAPPED + mapping z UI, retry/fallback, zatwierdzenia, pomiar p50/p99.
