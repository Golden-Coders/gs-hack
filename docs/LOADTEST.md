# Test obciążenia

Cel: zmierzyć przepustowość (msg/s), latencję end-to-end (`delivered_at − received_at`, p50/p99)
i lag konsumentów Kafki (gdzie jest wąskie gardło). Projekt: design D14.

Generator `loadgen` (`crates/loadgen`) pisze jednocześnie na `POST /ingest` i na topik źródłowy
(`SOURCE_TOPIC`, domyślnie `trades`), z mieszanką trzech schematów (`workflow.v1`, `alertmanager`,
`trade.rejected.v2`) i sterowanym odsetkiem duplikatów (żeby zmierzyć też dedup).

## Jak uruchomić

Wymagania: infrastruktura i binarki z `docker compose` (np. `docker compose up -d --build`).

Lokalnie (binarki z hosta, domyślne adresy `localhost`):

```bash
export CARGO_TARGET_DIR=/Users/kwojt/Documents/Programowanie/gs-hack/target   # opcjonalnie
RATE=500 DURATION_SECS=60 DUP_RATIO=0.5 MODE=both cargo run --release -p loadgen
```

W sieci compose (serwis `loadgen` z profilu `tools`; adresy wewnętrzne kontenerów):

```bash
docker compose --profile tools run --rm \
  -e INGEST_URL=http://ingest:8081 -e RATE=500 -e DURATION_SECS=60 loadgen
```

Zmienne środowiskowe:

| Zmienna | Domyślnie | Znaczenie |
|---|---|---|
| `RATE` | `100` | zadane tempo, msg/s (łącznie HTTP + Kafka) |
| `DURATION_SECS` | `10` | czas przebiegu |
| `DUP_RATIO` | `0.5` | odsetek zdarzeń z payloadem bez zmian (powtórka fingerprintu), 0.0–1.0 |
| `MODE` | `both` | `http`, `kafka` lub `both` (naprzemiennie) |
| `INGEST_URL` | `http://localhost:8081` | baza serwisu ingest; wysyłamy na `{INGEST_URL}/ingest` |
| `SOURCE_TOPIC` | `trades` | topik źródłowy Kafki (mapowany w `config/sources/`) |
| `KAFKA_BROKERS` | `localhost:19092` | z `common::Config` |

Na końcu przebiegu `loadgen` wypisuje: wysłane, błędy, zaplanowane, osiągnięte msg/s,
wygenerowane zdarzenia, odsetek duplikatów i liczbę unikalnych fingerprintów.

Ograniczenia generatora:
- Kafka niesie tylko `trade.rejected.v2` (jedyne źródło w seedzie to `trades`); HTTP rotuje wszystkie trzy schematy.
- Unikaty dostają sufiks `-<n>` (`n` startuje od czasu uruchomienia), więc kolejne przebiegi nie zlewają się z poprzednimi w dedupie.
- Wysyłki są w paczkach co 10 ms i czekają na zakończenie paczki; przy wolnym brokerze tempo spada poniżej zadanego (osiągnięte msg/s to wynik uczciwy).

## Jak zebrać p50/p99

Po przebiegu (system po przepływie, bez nowego ruchu, poczekać chwilę na dostarczenie). Zanotuj czas
startu przebiegu (UTC) i podaj go jako `since` – liczą się tylko dostarczenia z `received_at >= since`:

```bash
docker exec -i gs-hack-postgres-1 psql -U alerting -d alerting -v ON_ERROR_STOP=1 \
  -v since="'2026-11-21 02:00'" < crates/loadgen/latency.sql
```

`since` to wyrażenie SQL (stałą datę podajemy w apostrofach); bez `-v since=...` domyślnie
`now() - interval '15 minutes'`. Bez okna liczyłyby się stare fixtures i ponowne wysyłki
z dawnym `received_at` (na wspólnym stosie p99 wychodziło ~28 dni).

Skrypt zwraca p50/p99 w ms (`delivered_at − received_at` dla `status = 'delivered'`) oraz liczbę alertów
i sumę liczników z `alerts` (ta część bez okna – cała tabela).

## Jak zebrać lag konsumentów

```bash
crates/loadgen/lag.sh                 # domyślna lista grup
crates/loadgen/lag.sh router sender   # wybrane grupy
REDPANDA_CONTAINER=inny-redpanda-1 crates/loadgen/lag.sh   # inna nazwa kontenera
```

Skrypt woła `docker exec gs-hack-redpanda-1 rpk group describe <grupa>` (kontener z `REDPANDA_CONTAINER`,
działa z dowolnego katalogu) dla grup:
`lifecycle-normalizer`, `lifecycle-dedup`, `router`, `runbooks`, `sender`, `sender-retry`, `ingest-source-trades`.
Zwraca kod ≠ 0, gdy nie udało się opisać żadnej grupy (zły kontener, broker nie działa).
Lag zapisujemy w trakcie przebiegu (największa wartość na grupę) i po nim (ma spaść do zera).

## Wyniki

| Pomiar | Data | msg/s (wysłane / osiągnięte) | p50 [ms] | p99 [ms] | Max lag (grupa) | Konfiguracja | Komenda |
|---|---|---|---|---|---|---|---|
| Pomiar 1 (po 18.1) | 2026-11-21 02:10:49 CET | 500 / 499.9 (29 995 wysłanych, 0 błędów) | 26 070 (tylko email) | 40 163 (tylko email) | 9 954 (`sender`); `lifecycle-normalizer` 569, `lifecycle-dedup` 174, `router` 2 | commit `7b50707`; obrazy ingest/lifecycle/router/sender/api z 2026-11-21 00:30:10 UTC (sprzed PR #26 – api bez wymogu tokenu); 12 partycji/topik, po 1 instancji; `DUP_RATIO=0.5 MODE=both`; `RATE=500 DURATION_SECS=60` | `RATE=500 DURATION_SECS=60 DUP_RATIO=0.5 MODE=both INGEST_URL=http://localhost:8081 cargo run --release -p loadgen`; `latency.sql` z `since='2026-11-21 01:10:49+00'` |
| Pomiar 1 – rozgrzewka | 2026-11-21 02:10:07 CET | 100 / 99.9 (1 999, 0 błędów) | — (nie mierzono osobno) | — | 0 (wszystkie grupy po przebiegu) | commit `7b50707`; obrazy ingest/lifecycle/router/sender/api z 2026-11-21 00:30:10 UTC (sprzed PR #26 – api bez wymogu tokenu); 12 partycji/topik, po 1 instancji; `DUP_RATIO=0.5 MODE=both`; `RATE=100 DURATION_SECS=20` | `RATE=100 DURATION_SECS=20 DUP_RATIO=0.5 MODE=both INGEST_URL=http://localhost:8081 cargo run --release -p loadgen` |
| Pomiar końcowy (skalowanie ×2 — 2.3 & 3.1) | 2026-11-21 03:02:28 CET | 500 / 499.8 (14 995 loadgen + 80 sond SMS/UI, 0 błędów) | bez email: 123.4 (`sms` 125.1, `phone` 120.3, `ui` 122.8); `email`: 16.6 | bez email: 231.8 (`sms` 229.4, `phone` 224.4, `ui` 234.6); `email`: 186.2 | 28 (`sender`, 2 inst.); `router` 20, `lifecycle-dedup` 5, `lifecycle-normalizer` 2 → 0 na końcu | commit `f8dcd72` (obrazy zbudowane `2026-11-21T01:56:43Z`, po PR #32/#34); 12 partycji/topik; `--scale lifecycle=2 --scale sender=2`; `DUP_RATIO=0.5 MODE=both`; `RATE=500 DURATION_SECS=30` | `docker compose up -d --scale lifecycle=2 --scale sender=2 && docker compose --profile tools run --rm -e INGEST_URL=http://ingest:8081 -e RATE=500 -e DURATION_SECS=30 -e DUP_RATIO=0.5 -e MODE=both loadgen` + `latency.sql` (`since='2026-11-21 02:02:28+00'`) |

<!-- P2 6.4 Benchmark ewaluacji warunków (crates/condition, 100 reguł × zdarzenie, 1000 zdarzeń = 100 000 ewaluacji):
     Release: 10.76 ms łącznie | 10.76 µs / zdarzenie (100 reguł) | 107.6 ns / reguła (~92 900 zdarzeń/s przy 100 regułach)
     Debug (opt-level=1): 13.03 ms łącznie | 13.03 µs / zdarzenie (100 reguł) | 130.3 ns / reguła (~76 700 zdarzeń/s przy 100 regułach)
     Komenda: cargo test -p condition benchmark_100_rules_evaluation -- --nocapture -->

### Wnioski z pomiaru 1

Przebieg główny: 500 msg/s przez 60 s osiągnięte bez błędów po stronie `ingest`/Kafki. Lag (próbki co ~10 s, `lag.sh`):

| Czas | normalizer | dedup | router | runbooks | sender | ingest-source-trades |
|---|---|---|---|---|---|---|
| 02:11:00 | 569 | 11 | 2 | 0 | 1 412 | 215 |
| 02:11:13 | 454 | 28 | 2 | 3 | 3 954 | 2 |
| 02:11:25 | 4 | 3 | 0 | 1 | 6 264 | 3 |
| 02:11:36 | 5 | 4 | 1 | 1 | 8 103 | 3 |
| 02:11:47 (koniec ruchu) | 14 | 174 | 2 | 2 | 9 954 | 2 |
| 02:11:58 | 0 | 0 | 0 | 0 | 8 587 | 0 |
| 02:12:31 | 0 | 0 | 0 | 0 | 0 | 0 |

- **Wąskie gardło: `sender`.** Lag rośnie liniowo przez cały przebieg (~170 wiad./s) i spada do zera dopiero ~45 s po końcu ruchu
  (opróżnianie ~260 wiad./s). Lifecycle i router nadążają (chwilowy lag normalizera na starcie, potem ~0).
  `sender` przetwarza `deliveries.requested` ściśle sekwencyjnie (`crates/sender/src/lib.rs:40-70`: `recv` → `process_request(...).await` → commit),
  a każdy e-mail to nowe połączenie TCP i pełna sesja SMTP do Mailpit (`crates/sender/src/adapters.rs:292`, `run_smtp_client_session` :369)
  plus zapisy do PG i publikacje statusu. Przepustowość ≈ 200–260 dostarczeń/s na instancję niezależnie od 12 partycji.
- **Latencja:** wszystkie 22 483 dostarczenia z okna to `email` (reguły kierują ruch z `loadgen` na e-mail), więc wiersz
  „bez email” z `latency.sql` jest pusty; p50/p99 w tabeli to kanał email i są zdominowane przez kolejkę w `sender`
  (p50 26 s, p99 40 s), nie przez sam SMTP. Na pustym systemie (first-flow, sms/phone) e2e to ~30 ms.
- **Dedup poprawny (wyjaśnienie różnicy 3 alertów — zadanie 2.2):** suma liczników w `alerts` wzrosła dokładnie o liczbę wysłanych zdarzeń (2 582 → 4 581 → 34 576 = +1 999 +29 995),
  natomiast liczba nowych wierszy w `alerts` była mniejsza o dokładnie **3** od liczby unikalnych fingerprintów raportowanej przez `loadgen`
  (rozgrzewka: 1 019 nowych alertów vs 1 022 unikalnych fingerprintów; przebieg główny: 14 928 vs 14 931).
  **Przyczyna (brak utraty w potoku):** w `crates/loadgen/src/lib.rs` (`Generator::next_event`) przy `duplicate == true` generator wysyła niezmienione
  zdarzenia bazowe z 3 fixtures (`workflow.v1|order-settlement|validate-counterparty|TIMEOUT`, `alertmanager|HighErrorRate|payments-api|eu-west`,
  `trade.rejected.v2|T-998877`) i wlicza te 3 bazowe identyfikatory do `Generator::unique_identities()` dla bieżącego przebiegu.
  Ponieważ pomiar 1 uruchamiano na niepustej bazie (po wcześniejszych testach `first-flow.sh` / `appendix-a.sh`), te 3 bazowe alerty
  **już istniały w tabeli `alerts` w stanie `open`** — więc `lifecycle-dedup` poprawnie zdeduplikował zdarzenia duplikatowe na istniejących 3 alertach
  (podbijając ich `count`), zamiast tworzyć 3 nowe wiersze. Każdy z unikalnych sufiksów `-{now_ms}` (1 019 i 14 928) utworzył dokładnie 1 nowy alert;
  brak duplikatów `idempotency_key` w `deliveries`. W skryptach E2E na niepustej bazie (np. `resilience.sh`) należy oczekiwać `delta(alerts) == unique_identities`
  tylko na czystej bazie lub z `DUP_RATIO=0`, a przy `DUP_RATIO > 0` na niepustej bazie `delta(alerts) == unique_identities - (liczba już istniejących bazowych fingerprintów ∈ {0..3})`,
  podczas gdy `delta(sum(alerts.count)) == wysłane` zawsze zachodzi dokładnie.
- **Ruch obcy:** równolegle na stosie chodziły pojedyncze testy e2e innej sesji (pomijalne wobec 30 tys. zdarzeń).
- **Rekomendacja:** optymalizować `sender`: (1) przetwarzanie współbieżne — np. `split_partition_queue` / zadanie per partycja albo
  ograniczona współbieżność (`buffer_unordered`) z commitem offsetu po kolei; (2) pula/ponowne użycie połączeń SMTP zamiast
  połączenia na wiadomość; (3) na krótko `--scale sender=2..4` (19.G1) — grupa ma 12 partycji, więc skaluje się liniowo.
  Drugorzędnie: normalizer/dedup czekają na potwierdzenie każdej publikacji (`crates/lifecycle/src/normalizer.rs:93`,
  `crates/lifecycle/src/dedup.rs:464-485`) — przy wyższym RATE mogą być następne. Pomiarów 1000–2000 msg/s nie robiono
  (sender i tak się nie mieści przy 500).

Kolumny: msg/s z wyjścia `loadgen` (`osiągnięte msg/s`), p50/p99 z `latency.sql`, lag z `lag.sh`.
W „Konfiguracja” wpisz wartości `RATE`, `DURATION_SECS`, `DUP_RATIO`, `MODE` oraz wersję commita.

### Wnioski z pomiaru końcowego (`--scale lifecycle=2 --scale sender=2`, 2.3 & 3.1)

- **Skalowanie poziome działa liniowo (12 partycji → po 6 partycji na instancję):** `lifecycle-normalizer` (2), `lifecycle-dedup` (2) i `sender` (2) utrzymały stabilny, niemal zerowy lag przy 500 msg/s (`sender` max lag **28** vs **9 954** przy 1 instancji w pomiarze 1; na końcu przebiegu wszystkie grupy `TOTAL-LAG = 0`).
- **Latencja e2e w rozbiciu na kanały (`delivered_at − received_at`, `latency.sql`):**
  - Zbiorczo **bez email** (204 dostarczenia: `sms` 82, `phone` 82, `ui` 40): **p50 = 123.4 ms**, **p99 = 231.8 ms**.
  - **`email`** (11 235 dostarczeń): **p50 = 16.6 ms**, **p99 = 186.2 ms**.
  - **`phone`** (82): p50 = 120.3 ms, p99 = 224.4 ms | **`sms`** (82): p50 = 125.1 ms, p99 = 229.4 ms | **`ui`** (40): p50 = 122.8 ms, p99 = 234.6 ms.
- **Dedup i idempotencja przy 2 instancjach:** `delta(alerts) = 7 590` (dokładnie `7 510` unikalnych z `loadgen` + `80` sond SMS/UI), `delta(sum(alerts.count)) = 15 075` (dokładnie `14 995 + 80`), **0 duplikatów** `idempotency_key` w `deliveries`.
