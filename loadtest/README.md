# Testy obciążenia – szukanie maksymalnej przepustowości i wąskich gardeł

Uzupełnia `docs/LOADTEST.md` (pojedynczy przebieg `loadgen`). Tu: **test schodkowy** (rosnące tempo aż do
załamania), pomiar etapów potoku i zasobów, automatyczna klasyfikacja kroków.

## Izolacja od stosu deweloperskiego

Test działa w osobnym projekcie compose `gsload` z własnym obrazem `alerting-backend:loadtest`,
bez portów na hoście (nie koliduje z `docker compose up` deweloperskim, nie rusza jego bazy).
Ruch generują kontenery `loadgen` wewnątrz sieci compose (bez narzutu forwardingu portów Docker Desktop).

```bash
C="docker compose -p gsload -f docker-compose.yml -f loadtest/docker-compose.loadtest.yml"
$C build backend-build              # po każdej zmianie kodu
$C up -d                            # opcjonalnie: --scale lifecycle=2 --scale sender=2
loadtest/step_load.py --rates 500,1000,2000,4000,8000 --step-secs 30
$C down -v                          # sprzątanie (ważne: baza rośnie z każdym krokiem)
```

Uwaga: pamięć VM-a Dockera (domyślnie ~4 GB) jest wspólna – zatrzymaj stos deweloperski na czas testu.
Przed każdym nowym przebiegiem na „czystych” danych: `$C down -v && $C up -d`.

## Co mierzy `step_load.py`

Dla każdego kroku: tempo wysłane/osiągnięte, błędy HTTP/Kafka, **lag** każdej grupy konsumenckiej (max w trakcie ruchu),
**przepustowość każdego etapu** (Δ committed offset / Δt – wskazuje etap, który nie nadąża), CPU/RAM kontenerów,
czas **drain** po końcu ruchu, zliczone zdarzenia (`sum(alerts.count)` musi = wysłane), p50/p99 e2e per kanał.

Werdykt kroku:
- `SUSTAINED` – ≥95% zadanego tempa, ≤1% błędów, zaległość na końcu ruchu ≤3 s ruchu, drain ≤30 s,
- `DEGRADED` – ruch przyjęty, ale przetwarzanie nie nadąża (rośnie kolejka),
- `FAILED` – nie osiągnięto tempa lub błędy/timeouty/brak drainu.

Wyniki surowe: `loadtest/results/*.json`. Podsumowanie i wnioski: `loadtest/RESULTS.md`.

Parametry: `--per-loadgen` (maks. tempo na instancję generatora; powyżej uruchamiane są kolejne, z rozłącznymi
identyfikatorami przez `LOADGEN_ID`), `--dup-ratio`, `--mode http|kafka|both`, `--no-stop`.
