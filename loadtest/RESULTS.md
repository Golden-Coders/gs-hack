# Wyniki testów obciążenia (2026-11-21)

Narzędzia i metodyka: `loadtest/README.md`. Surowe dane: `loadtest/results/*.json`.

## Środowisko i metoda

- Docker Desktop na macOS: **14 CPU, ~4 GB RAM dla całego VM-a** (broker, Postgres, wszystkie serwisy i generatory ruchu na tej samej maszynie – wyniki to dolne oszacowanie, nie benchmark produkcyjny).
- Świeży obraz z bieżącego kodu (branch `p4/ui-primitives`), izolowany projekt compose `gsload`, czysta baza przed każdą konfiguracją.
- Redpanda `--smp=1` (domyślnie), Postgres 16 z domyślnym strojeniem (`max_connections=100`, `shared_buffers=128MB`), 12 partycji/topik.
- Ruch: `loadgen` w sieci compose, schodki po 30 s, `DUP_RATIO=0.5`, mieszanka 3 schematów. `MODE=both` = połowa HTTP, połowa Kafka (`trades`); `MODE=http` = tylko `POST /ingest`.
- Jedno zdarzenie → średnio ~0,5 nowego alertu → ~0,75 dostawy (email do Mailpit), więc „msg/s” to zdarzenia wejściowe.
- Werdykt kroku: **SUSTAINED** = ≥95% tempa, ≤1% błędów, zaległość ≤3 s ruchu, drain ≤30 s; **DEGRADED** = ruch przyjęty, ale kolejki rosną; **FAILED** = brak drainu w 180 s.

## Główny wynik

| Konfiguracja | Maks. utrzymywalne | Co pęka pierwsze |
|---|---|---|
| 1 instancja każdego serwisu (`MODE=both`) | **450 msg/s** (twardy sufit ≈ 440) | `lifecycle-normalizer` |
| lifecycle×4 (`both`) | < 1000 (1000 = DEGRADED) | `router` (~330/s), konsument źródła Kafki w `ingest` |
| lifecycle×4, router×4, sender×2, runbooks×2, ingest×2 (`both`) | < 1500 | `ingest-source-trades` (~360–450 msg/s) |
| j.w., `MODE=http` | **1500 msg/s** (3000 = DEGRADED) | `lifecycle-normalizer` (~1500/s przy 4 inst.) |
| j.w. + Redpanda `--smp=4` (`http`) | **1500 msg/s** – bez zmiany | `lifecycle-normalizer` (broker nie jest limitem) |
| lifecycle×12, router×4, sender×2, runbooks×2, ingest×2, PG `max_connections=400`, Redpanda `--smp=4` (`http`) | **brak SUSTAINED już przy 3000** (3000 → 10 s zaległości) | `lifecycle-dedup` (~2600/s), `runbooks` (~550/s), `router`/`sender` (~1200/s) |

**Maksymalna przepustowość systemu na tej maszynie: ≈ 450 msg/s w konfiguracji domyślnej; ≈ 1500 msg/s (HTTP) po skalowaniu poziomym; ≈ 2500–3000 msg/s to szczyt przetwarzania w najlepszej konfiguracji, utrzymywany tylko z rosnącą kolejką.** Przyjmowanie zdarzeń (ingest → `events.raw`) nie jest limitem: ruch 10 000 msg/s został przyjęty bez ani jednego błędu HTTP/Kafka – to kolejki za ingestem się zapełniają.

### Punkt załamania

Nie następuje „twarde” odrzucanie ruchu, tylko **degradacja przez kolejkowanie**:
1. Powyżej sufitu etapu lag rośnie liniowo, a latencja e2e z ~10–40 ms skacze do sekund (p50 2 s przy 500 msg/s w konfiguracji 1×; 18 s przy 1000; 53 s przy 5000 w skalowanej).
2. Przy bardzo wysokim tempie przepustowość etapów **spada** zamiast się utrzymać (normalizer 1660 → 620 msg/s, dedup 1220 → 513 msg/s przy 5000 → 8000 msg/s): kolaps pod przeciążeniem.
3. Przy 8000–10 000 msg/s drain nie kończy się w 180 s (zaległość 200–260 tys. zdarzeń = 25 s ruchu).
4. W teście z `--smp=4` kontener Redpandy został zabity (exit 137, prawdopodobnie OOM przy 4 GB VM i 1,5 GB pamięci brokera) po kroku 10 000 msg/s – to artefakt mojej konfiguracji testowej, ale pokazuje brak ochrony przed przeciążeniem pamięci.

## Wąskie gardła (w kolejności ujawniania się)

| # | Wąskie gardło | Dowód | Przyczyna w kodzie |
|---|---|---|---|
| 1 | **`lifecycle-normalizer` – sekwencyjna pętla** | 1×: ~440 msg/s przy CPU lifecycle 26% (latency-bound, nie CPU); lag normalizera 1,8–15 tys. przy 500–1000 msg/s. Skaluje się liniowo z liczbą instancji (×4 → ~1500, ×12 → ~3000+) | `crates/lifecycle/src/normalizer.rs:40-65`: `recv → await publish(acks=all) → commit` na jednym zadaniu; sufit = 1/RTT publikacji, niezależnie od 12 partycji |
| 2 | **`router` – ten sam wzorzec** | po usunięciu #1: lag routera 4,6–7,6 tys., przepustowość ~330/s na instancję | `crates/router/src/lib.rs:59-82`: sekwencyjnie, z `await publish` na każdy `DeliveryRequest` |
| 3 | **Konsument źródeł Kafki w `ingest`** | `ingest-source-trades` lag 8,4–26 tys., ~360–450 msg/s mimo ×2 instancji; przy `MODE=http` lag = 0 | `crates/ingest/src/sources.rs:94-146`: jedno zadanie per źródło, `recv → publish → commit` |
| 4 | **`lifecycle-dedup`** | ~2600/s przy ×12 i **spadek** pod większym tempem (2750 → 2630 → 2000 → 1640); lag 8–33 tys. | `crates/lifecycle/src/dedup.rs:440-480`: pętla sekwencyjna (cache w pamięci i flush wsadowy działają dobrze); dodatkowo klucz = fingerprint, więc zdarzenia jednego alertu trafiają do jednej partycji (**hot key** – niezweryfikowane osobnym pomiarem, patrz niżej) |
| 5 | **`runbooks`** | lag 17–24 tys., ~270/s na instancję, najwolniejszy etap za routerem przy ×12 | pojedyncza pętla konsumenta; do zbadania profilem |
| 6 | **Postgres** | 150–220% CPU przy 1500–5000 msg/s, 100+ połączeń; baza jest wspólnym zasobem wszystkich serwisów | brak PgBouncera, domyślne strojenie |
| 7 | **Limit połączeń PG (twardy)** | przy lifecycle×12 + router×4 + … Postgres odrzucał połączenia (`too many clients already`, nawet z psql) | `crates/common/src/db.rs:32`: `max_connections(10)` na proces × ~23 procesy > `max_connections=100` |

### Co NIE jest wąskim gardłem
- **Ingest HTTP** – przyjął 10 000 msg/s bez błędów.
- **Redpanda** – z `--smp=1` pracuje na 80–95% CPU, ale `--smp=4` (250% CPU) nie podniosło przepustowości; limit leży w aplikacji.
- **`sender`** – po PR #38 (współbieżność) nie limituje (p50 e-mail 10–40 ms w nasyconym stanie niższych kroków); Mailpit ~50–70% CPU przy 1500–5000 msg/s, więc przy większym ruchu e-mail stanie się limitem tylko dla mocka.
- **Dedup poprawność** – w każdym kroku `sum(alerts.count)` = liczba wysłanych zdarzeń (0 strat, brak duplikatów dostaw), także w przeciążeniu.

## Rekomendacje (kolejność wg zysku)

1. **Normalizer, router, ingest-source, dedup: przetwarzanie współbieżne z commitem po kolei** – jak w `sender` (`buffer_unordered` per partycja + okno offsetów `mark_done`). Spodziewany zysk: wielokrotność 440 msg/s na instancję bez kosztu dodatkowych procesów (CPU leży odłogiem: 25%).
2. **Dla publikacji w normalizerze/routerze nie czekać na ack pojedynczo** – wsadowo (`producer.send` bez `await` per wiadomość, flush przed commitem offsetu); zachowuje at-least-once.
3. **Partycjonowanie `trades`** i skalowanie `ingest` (konsument źródła jest pojedynczy per grupa).
4. **PgBouncer lub niższy `max_connections` na proces** (pula 10 × N procesów przekracza limit PG przy skali > ~8 instancji); strojenie PG (`shared_buffers`, `synchronous_commit` dla ścieżek niekrytycznych).
5. **Backpressure / limity**: ingest przyjmuje bez ograniczeń, a lag rośnie do minut – rozważyć odpowiedź 429/503 powyżej progu lagu albo alert na lag; ustawić limity pamięci brokera.

## Czego nie zmierzono (uczciwie)

- **Skew klucza (hot fingerprint)** i profile `DUP_RATIO=1.0` / `0.0` – eksperyment przerwany po padzie Redpandy (OOM); hipoteza o hot partition w dedupie jest tylko wnioskiem z kodu i kształtu ruchu (duplikaty to 3 bazowe fingerprinty).
- **Wydajność na realnym sprzęcie / wielu węzłach** – całość na jednym VM-ie 4 GB, generator ruchu konkuruje o CPU z systemem.
- **Kanały zewnętrzne** – email do Mailpit, SMS/phone to mocki; realna latencja dostawców nie jest uwzględniona.
- Przy `MODE=both` wynik 1500 msg/s w konfiguracji ×4 nie był osiągnięty (SUSTAINED tylko przy `http`).
- Wyniki to pojedyncze przebiegi (30 s na krok), bez powtórzeń i przedziałów ufności; różnice rzędu 10% traktować jako szum.

## Jak powtórzyć

```bash
loadtest/reset.sh --scale lifecycle=4 --scale router=4 --scale sender=2 --scale runbooks=2 --scale ingest=2
loadtest/step_load.py --rates 1000,1500,2500,4000 --step-secs 30 --mode http
# parametry infrastruktury: RP_SMP, RP_MEM, PG_MAXCONN, PG_SHARED_BUFFERS (patrz loadtest/docker-compose.loadtest.yml)
```

---

# Po optymalizacji (gałąź `perf/consumer-concurrency`, 2026-11-21)

Te same warunki co wyżej (VM 4 GB, Redpanda `--smp=1`, PG domyślny, 12 partycji, `DUP_RATIO=0.5`, 30 s na krok,
czysta baza przed każdą konfiguracją: `loadtest/reset.sh`; obraz: `loadtest/build.sh`). Surowe dane: `loadtest/results/`
(`before-*`, `stage*`, `after-*`, `final-1x-*`).

## Co zmieniono

| Etap | Zmiana | Commit |
|---|---|---|
| 0 | Wspólny moduł `common::kafka::concurrent`: pasy per `hash(klucz)` (sekwencyjnie w pasie, kolejność per klucz także między topikami jednej subskrypcji), commit ściśle po kolei per (topik, partycja) po `Ok` handlera, bariera rewokacji w `pre_rebalance` (pasy kończą kolejkę → `on_revoke` → commit synchroniczny → dopiero wtedy oddanie partycji), drenowanie przy shutdownie, semafor `max_in_flight`; `OffsetTracker` przeniesiony z `sender` | `ce9e5b5` |
| 1 | `lifecycle-normalizer` na module (32 pasy, `NORMALIZER_*`) | `f7122d2` |
| 2 | `router` na module (32 pasy, `ROUTER_*`) + publikacja wszystkich `DeliveryRequest` jednej wiadomości współbieżnie (`try_join_all`, `Ok` po komplecie ack) | `ffe665b` |
| 3 | Konsument źródeł Kafki w `ingest` na module (32 pasy, `INGEST_SOURCE_*`), łagodny restart źródła (sygnał + drenowanie zamiast `abort`), `trades` w `topics-init` z 12 partycjami, ostrzeżenie przy za małej liczbie partycji | `857dcfb` |
| 4 | `lifecycle-dedup` na module: jedna instancja `Dedup` (własny cache) na pas, flush co 1 s w `on_tick`, flush + czyszczenie cache w `on_revoke` (przed oddaniem partycji – wcześniej po fakcie) | `b58dffd` |
| 5 | `runbooks`: pomiar + testy, bez zmiany pętli (uzasadnienie niżej) | `40e44a8` |
| 6 | Pula PG konfigurowalna (`DB_MAX_CONNECTIONS`/`DB_MIN_CONNECTIONS`), domyślnie per serwis: lifecycle 10, sender 8, runbooks 6, api 5, router 2, ingest 2 (budżet lifecycle×4 itd. = 93/100); PgBouncer opisany w `docs/LOADTEST.md` | `7b39e5c`, `3ad7e34` |
| – | `loadgen`: klucz wiadomości na `trades` = tożsamość zdarzenia (wcześniej stała nazwa schematu ⇒ cały ruch Kafki w 1 partycji i 1 pasie) | `21885f7` |
| – | Poprawki po review (pasy nie kręcą się po porzuceniu dispatchera, panika handlera = ponowienie, wykładniczy backoff do 5 s, `catch_unwind` w callbacku rebalansu, `drain_timeout` źródła < `STOP_TIMEOUT`) | `41020a3` |

## Przed / po – 1 instancja każdego serwisu, `MODE=both`

| Tempo [msg/s] | Przed (`before`) | Po etapach 0+1+2+6 | + etap 4 (dedup) | + klucz loadgen | **Końcowo (`final-1x`)** |
|---|---|---|---|---|---|
| 450 | SUSTAINED (lag normalizera 123) | SUSTAINED | DEGRADED* | SUSTAINED | **SUSTAINED** (max lag 14) |
| 500 | DEGRADED (normalizer 428/s, lag 2 027, p50 2,5 s) | SUSTAINED | – | – | – |
| 600 | DEGRADED (lag 12 088, p50 36 s) | SUSTAINED | SUSTAINED | SUSTAINED | **SUSTAINED** |
| 800 | – | DEGRADED (dedup 650/s) | SUSTAINED | SUSTAINED | **SUSTAINED** |
| 1000 | – | DEGRADED (dedup 610/s, lag 10,9 tys.) | SUSTAINED | SUSTAINED | **SUSTAINED** (p50 11 ms, p99 44 ms) |
| 1500 | – | – | DEGRADED (`ingest-source` 430/s) | SUSTAINED | **SUSTAINED** (p50 12 ms, p99 187 ms) |
| 2000 | – | – | – | DEGRADED (3,97 s zaległości) | **na granicy**: SUSTAINED (2,27 s zaległości, próg 3 s); w powtórzeniu `after-1x` DEGRADED (3,65 s) |
| **Maks. SUSTAINED** | **450** | 600 | 1000 | 1500 | **1500 stabilnie, 2000 na granicy** |

\* pierwszy krok tego przebiegu zakłócony: równolegle chodziły testy integracyjne subagentów na tym samym VM
(deweloperska Redpanda/PG); powtórzenie na czystym VM – SUSTAINED.

Poprawność w każdym przebiegu: `zliczone zdarzenia` (`sum(alerts.count)`) = wysłane, **0 duplikatów `idempotency_key`**
w `deliveries`, wszystkie grupy lag = 0 po drainie, 31–37 połączeń PG (1×), 88–93 (skalowane).

## Skalowanie i profile ruchu (po optymalizacji)

| Konfiguracja | Wynik | Co pęka |
|---|---|---|
| lifecycle×4, router×4, sender×2, runbooks×2, ingest×2, `MODE=http`: 1500 / 3000 / 5000 | 1500 SUSTAINED; 3000 i 5000 DEGRADED (13 s / 20 s zaległości, drain ~61 s) | normalizer już NIE (2 922 / 4 996 msg/s; przed: ~1 500); limitem są `runbooks` (359 → 191 msg/s na 2 instancjach), `lifecycle-dedup` (~2 000–2 300/s łącznie), `sender` i **Postgres 188–222% CPU** |
| j.w., `DUP_RATIO=1.0` (sztorm duplikatów, 3 gorące fingerprinty), 3000 | **SUSTAINED** (lag ≤ 1 009, drain 3,4 s; 3 alerty, `sum(count)` = 89 976 = wysłane) | gorący klucz nie jest problemem: duplikat = inkrement w cache pasa |
| j.w., `DUP_RATIO=0.0` (same nowe alerty), 3000 | DEGRADED (37 s zaległości, drain 53 s; alerty = wysłane = 89 973) | `runbooks` (526 msg/s), `sender` (1 539/s), **Postgres 276–327% CPU**; dedup 3 194/s (każde zdarzenie = transakcja PG) |

Przed optymalizacją ten sam profil skalowany dawał 1500 SUSTAINED / 3000 DEGRADED z normalizerem jako limitem;
profile `DUP_RATIO=1.0/0.0` się nie wykonały (pad Redpandy). Teraz żaden kontener nie padł (bez exit 137).

E2E na obrazie z gałęzi (osobny projekt `gse2e`, nie stos deweloperski – patrz niżej): `appendix-a.sh` (D1 + D2: dedup 500,
reopen), `runbooks.sh`, `first-flow.sh`, `resilience.sh` (restart ingest/lifecycle/router/sender w trakcie ruchu:
`delta(sum_count)` = przyjęte, 0 duplikatów, fallback i DLQ) – **wszystkie PASS**.

## Nowe wąskie gardła

1. **1 instancja:** `ingest-source-trades` (~750–830 msg/s przy 1000 msg/s na Kafce, lag 5–7 tys. przy 2000) i **Postgres**
   (142–172% CPU przy 2000). Przyczyny w konsumencie źródła nie zbadano profilem (32 pasy, klucze rozłożone na 12 partycji –
   prawdopodobnie koszt `accept` + jeden producent na proces); obejście: `--scale ingest=N`.
2. **Skalowane:** `runbooks` i **Postgres** jako wspólny zasób (CPU 200–330%). Przy samych nowych alertach każde zdarzenie
   to transakcja w dedupie, run runbooka (~20 zapytań) i zapis dostawy.

## Czego nie poprawiono (i dlaczego)

- **`runbooks` (etap 5) – bez zmian w pętli.** Pomiar: niedopasowany alert = 0 zapytań PG (sama migawka), dopasowany =
  ~5 zapytań na utworzenie runu + ~15 w odłączonym `advance`. Wersja z pasami (16) była przy puli 6 **wolniejsza**
  (90–123/s vs ~200/s), przy puli 10 – w szumie. Limitem jest pula PG dzielona z nieograniczoną liczbą zadań `advance`,
  nie czekanie konsumenta. Zostały testy (kolejność komend per run, idempotencja runu, commit po skutkach) i benchmark
  (`crates/runbooks/src/it.rs`). Dalej: semafor na `advance`, kompilacja warunków raz na migawkę, leniwy `alert_context`,
  skalowanie instancjami.
- **Postgres** – nie stroiliśmy (`shared_buffers`, `synchronous_commit`) ani nie wprowadziliśmy PgBouncera (opisany jako opcja).
- **Kolejność firing → resolved tego samego fingerprintu w ciągu milisekund** (z review): `events.raw` jest kluczowany
  `event_id`, więc dwa zdarzenia jednego fingerprintu mogą trafić do różnych pasów/partycji normalizera i zamienić się
  miejscami; „resolved” przed otwarciem alertu jest pomijany w dedupie. Ryzyko istniało już przed zmianą (wiele partycji
  `events.raw`, skalowanie normalizera), teraz okno jest szersze. Poprawka wymaga zmiany semantyki dedupa (np. tombstone
  „resolved bez alertu” z czasem zdarzenia) – poza zakresem.
- **D4 (dedup):** offset duplikatu jest commitowany przed flushem inkrementu (≤ 1 s inkrementów do utraty przy awarii
  procesu) – bez zmian; przy rebalansie i shutdownie jest teraz lepiej (flush przed oddaniem partycji).
- **Backpressure na ingeście** (429/503 powyżej progu lagu) – nie robione.
- Pojedyncze przebiegi, 30 s na krok, bez przedziałów ufności (2000 msg/s przy 1× raz SUSTAINED, raz DEGRADED).

## Uwagi środowiskowe

- Stos deweloperski (`gs-hack`, obrazy `alerting-backend:dev`) działa na kodzie z innej gałęzi (jego baza ma migracje
  `1000+`), dlatego e2e na nowym kodzie uruchomiono w osobnym projekcie `gse2e` z obrazem `alerting-backend:loadtest`
  i portami hosta (przy zatrzymanym stosie deweloperskim), a potem stos deweloperski przywrócono (`docker compose start`).
- `cargo` na hoście wymaga `cmake` (budowa `rdkafka-sys`); doinstalowano przez Homebrew.
