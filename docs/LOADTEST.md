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
| `SOURCE_TOPIC` | `trades` | topik źródłowy Kafki (mapowany w `config/p1/sources/`) |
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
| Pomiar 1 (po 18.1) | | | | | | | |
| Pomiar końcowy | | | | | | | |

<!-- P2 6.4 Benchmark ewaluacji warunków (crates/condition, 100 reguł × zdarzenie, 1000 zdarzeń = 100 000 ewaluacji):
     Release: 10.76 ms łącznie | 10.76 µs / zdarzenie (100 reguł) | 107.6 ns / reguła (~92 900 zdarzeń/s przy 100 regułach)
     Debug (opt-level=1): 13.03 ms łącznie | 13.03 µs / zdarzenie (100 reguł) | 130.3 ns / reguła (~76 700 zdarzeń/s przy 100 regułach)
     Komenda: cargo test -p condition benchmark_100_rules_evaluation -- --nocapture -->

Kolumny: msg/s z wyjścia `loadgen` (`osiągnięte msg/s`), p50/p99 z `latency.sql`, lag z `lag.sh`.
W „Konfiguracja” wpisz wartości `RATE`, `DURATION_SECS`, `DUP_RATIO`, `MODE` oraz wersję commita.
