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
