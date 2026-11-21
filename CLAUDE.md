# CLAUDE.md — Alerting Service (hackathon, 4 osoby, 18 h)

Serwis alertingowy: zdarzenia w dowolnym schemacie → alerty z deduplikacją → reguły → powiadomienia na preferowanych kanałach → runbooki z akcjami i zatwierdzeniami. Zadanie: `docs/exercise.md`.

## Źródła prawdy (czytaj przed pracą)

Specyfikacja systemu i plan w OpenSpec:

| Gdzie | Co zawiera |
|---|---|
| `openspec/specs/<capability>/spec.md` | **aktualne wymagania i scenariusze** (= przypadki testowe) dla 10 capabilities |
| `openspec/changes/alerting-service-hardening/` | **aktywny change**: pozostała praca po MVP (odporność, skalowanie, pomiar końcowy, weryfikacja UI, demo, otwarte kwestie) |
| `openspec/changes/archive/2026-11-21-alerting-service-mvp/` | zarchiwizowane MVP: `proposal.md` (zakres, co odłożone), `design.md` (decyzje D1–D15: topologia Kafki, single writer, idempotencja, bezpieczeństwo akcji), `tasks.md` (historia 87 zadań, sekcja 19 = pipeline P1+P2), `team.md` (własność katalogów/tabel/topików) |

Implementację prowadź przez `/openspec-apply-change alerting-service-hardening`, ograniczając się do sekcji swojego strumienia. Odhaczaj zadania w `tasks.md` (`- [x]`) dopiero po spełnieniu weryfikacji opisanej w zadaniu. Zmiana zachowania systemu = delta spec w change'u, a po zakończeniu `/openspec-archive-change`.

## Zespół i własność

| Strumień | Osoba | Sekcje `tasks.md` | Piszesz tylko w |
|---|---|---|---|
| P1 Ingest i cykl życia | Krzysztof Jet | 2, 3, 4, 5 | `crates/ingest`, `crates/lifecycle`, `crates/loadgen`, `crates/api/src/p1.rs`, `migrations/1xxx_p1_*`, `config/p1/`, `docs/LOADTEST.md` |
| P2 Reguły, routing, dostarczanie | Krzysztof Wojt | 6, 7, 8, 9 | `crates/condition`, `crates/router`, `crates/sender`, `crates/api/src/p2.rs`, `migrations/2xxx_p2_*`, `config/p2/` |
| **P1+P2 pipeline** (od 2026-11-21) | Krzysztof Jet, Krzysztof Wojt | MVP 19 → hardening 2, 3, 6 | katalogi P1 i P2 + `crates/api/src/stream.rs`, `scripts/e2e/` |
| P3 Runbooki i akcje | Kostek | 10, 11, 12 | `crates/runbooks`, `crates/mock-targets`, `crates/api/src/p3.rs`, `migrations/3xxx_p3_*`, `config/p3/` |
| P4 UI, dokumentacja, demo | Damian (TypeScript, bez Rusta) | 13–17 | `ui/`, `README.md`, `docs/MOCKS.md`, `docs/DEMO.md`, `config/demo/`, cel `demo` w `Makefile` |

Numery sekcji w tabeli dotyczą zarchiwizowanego MVP. W `alerting-service-hardening`: sekcje 1 i 5 — wszyscy, 2, 3, 6 — P1+P2, 4 — P4.

**Jeśli nie wiesz, czyj jest strumień, w którym pracujesz — zapytaj użytkownika, zanim cokolwiek zmienisz.**

## Złote zasady (obowiązują każdego agenta)

1. **Edytuj tylko katalogi swojego strumienia** (tabela wyżej). Cudzy kod tylko czytasz.
2. **Zamrożone (zmiany tylko addytywne, po uzgodnieniu z zespołem):** `crates/contracts/`, `crates/common/`, `contracts/openapi.yaml`, `crates/api/src/main.rs`, `crates/api/src/state.rs`, `[workspace.dependencies]` w `Cargo.toml`, `docker-compose.yml`, `Dockerfile`. Addytywne = nowe pole jako `Option` z `#[serde(default)]`, nowy endpoint, nowy wariant na końcu enuma. Jeśli kontrakt nie wystarcza — **zatrzymaj się i opisz potrzebną zmianę**, nie obchodź go.
3. **Single writer:** stan alertu zmienia tylko `lifecycle` (P1), stan runbooka tylko `runbooks` (P3). Inni wysyłają komendy na `alerts.commands` / `runbooks.commands`.
4. **Zależności Rust:** wersje tylko w `[workspace.dependencies]`; w crate'cie `nazwa = { workspace = true }`.
5. **At-least-once:** offset Kafki commituj PO zapisaniu skutków (`common::kafka::commit`). Każdy skutek uboczny musi być idempotentny — klucze w `contracts::keys`.
6. **Bezpieczeństwo akcji:** wykonujemy tylko akcje z rejestru; `risk: high` zawsze wymaga zatwierdzenia; allowlista hostów (`ACTION_HOST_ALLOWLIST`); szablony bez dostępu do plików/env; sekrety tylko jako `${ENV}`.
7. **Testuj swój etap niezależnie** — fixtures z `crates/contracts/fixtures/` publikuj na swój topik wejściowy, czytaj wyjściowy. Nie czekaj na innych.
8. Małe, częste merge'e do `main` z zielonym `cargo test -p <twój crate>` (UI: `npm run typecheck`).

## Architektura w skrócie

```
HTTP /ingest(/{schema}) --+
                          +--> events.raw --> [lifecycle: normalizer] --> events.normalized (key=fingerprint)
Kafka (źródła z config) --+                         |  brak mappingu --> events.unmapped
                                                    v
                         alerts.commands --> [lifecycle: dedup + stan] --> alerts.changed
                                                                              |            \
                                                                          [router]      [runbooks] --> runbooks.commands (z UI)
                                                                              |              | escalate/notify
                                                                              v              v
                                                                     deliveries.requested --> [sender] --> UI / Email / SMS / Phone
                                                                                             retry --> deliveries.retry --> DLQ
P1/P2/P3 --> ui.events --> [api] --SSE /stream--> UI
```

Stos: Rust (tokio, axum, rdkafka, sqlx, minijinja), Kafka API (lokalnie Redpanda), Postgres 16, Mailpit (mock SMTP), UI: React + Vite + TypeScript. **Bez Redisa.**

## Konwencje

- **Migracje:** `migrations/<wersja>_<pN>_<opis>.sql`; P1 numeruje 1001–1999, P2 2001–2999, P3 3001–3999. Uruchamiane przez każdą binarkę przy starcie (`common::db::connect_and_migrate`). Bez kluczy obcych między tabelami różnych strumieni.
- **Konfiguracja:** pliki w `config/pN/` to seed + hot reload; w runtime źródłem prawdy jest Postgres (`NOTIFY` → przeładowanie → `ArcSwap`). Formaty = typy z `crates/contracts/src/config.rs`, przykłady w `crates/contracts/fixtures/config/`.
- **Warunki i szablony:** zawsze przez crate `condition` (P2). Do czasu prawdziwej implementacji to zaślepka (`evaluate` → `false`).
- **API dla UI:** gateway `api` (:8080), endpointy w `p1.rs`/`p2.rs`/`p3.rs` zgodnie z `contracts/openapi.yaml`. Tożsamość: nagłówek `X-Person-Id`; token: `Authorization: Bearer dev-token`.
- **Kontrakt OpenAPI zmieniony?** P4 regeneruje typy: `cd ui && npm run gen:api`.
- Język: dokumenty i komentarze po polsku, identyfikatory w kodzie po angielsku.

## Komendy

```bash
docker compose up --build                 # całość: UI :5173, API :8080, Mailpit :8025, mock-targets :8090
API_PORT=8088 docker compose up --build   # gdy port 8080 jest zajęty
docker compose up -d redpanda postgres mailpit topics-init   # tylko infrastruktura do pracy lokalnej
cargo run -p <binarka>                    # binarka lokalnie (domyślne env pasują do compose: localhost:19092, localhost:5432)
cargo test -p <crate>                     # testy swojego crate'a
cargo check --workspace                   # przed pushem

docker compose exec redpanda rpk topic list
docker compose exec redpanda rpk topic produce alerts.changed < crates/contracts/fixtures/topics/alerts.changed.json
docker compose exec redpanda rpk topic consume deliveries.requested -n 1
docker compose exec postgres psql -U alerting

cd ui && npm install && npm run mock      # mock API z OpenAPI na :4010
cd ui && npm run dev                      # UI na :5173 (VITE_API_URL w ui/.env, wzór: ui/.env.example)
```

Zmienne środowiskowe binarek: `crates/common/src/config.rs` (`DATABASE_URL`, `KAFKA_BROKERS`, `CONFIG_DIR`, `SMTP_HOST`, `ACTION_HOST_ALLOWLIST`, `API_TOKEN`, `<BIN>_PORT`, `RUST_LOG`).

## Ustalenia, które łatwo pomylić

- Koperta wejściowa: `{format, schema, payload}`; `format` to format payloadu (`json`, `yaml`), nie typ zdarzenia.
- Mapping (per `schema`) definiuje: pola, domyślne severity, fingerprint, warunek `resolved`. Reguła może nadpisać tylko severity i prezentację.
- Brak mappingu → zdarzenie UNMAPPED (nie błąd); po dodaniu mappingu ponowne przetworzenie.
- Dedup: to samo fingerprint łączy się z alertem do RESOLVED; potem **reopen tego samego alertu** (nowy epizod) i ponowne powiadomienie. Zmiany licznika idą na `ui.events`, nie na `alerts.changed`.
- Reguła wskazuje **KTO** (osoby/zespoły); **JAK** (kanały) wynika z preferencji osoby (zespół = domyślne). UI jest zawsze ostatnim kanałem zapasowym.
- Porażka dostarczenia = tylko błąd techniczny kanału (nieodebrany telefon to nie porażka).
- Runbook AUTOMATIC: sekwencyjnie, start od razu raz na epizod; krok manual/approval wstrzymuje; błąd zatrzymuje (ponów/pomiń/tryb ręczny); `if` to tylko tekst. Runbook MANUAL: tekst/Markdown dołączany do powiadomień (link w SMS). Nieznana akcja → krok manualny + licznik brakujących akcji.
- Zatwierdzanie tylko z UI.

## Odłożone (nie implementuj bez prośby)

Eskalacja przy braku ack, „Zapisz jako runbook” z incydentu, propozycje zmian do alertów (wymóg 2.4 zadania — ryzyko), LLM (w MVP tylko interfejs `RunbookAuthor` z `none`), edytory runbooków i osób w UI, warunki `if` jako predykaty, grupowanie/throttling, on-call.

## Znane otwarte kwestie

- design.md D10 zakłada `rule_ids` w `AlertChanged`, ale lifecycle nie zna reguł. Propozycja: `runbooks` czyta warunek przypiętej reguły z tabeli `rules` tylko do odczytu (tak jak router czyta runbooki MANUAL). Do potwierdzenia przez P2/P3.
- SSE `/stream` w `api` (konsumpcja `alerts.changed` + `ui.events`) to nadal szkielet z Fazy 0; realizacja w pipeline P1+P2, etap 19.B.
