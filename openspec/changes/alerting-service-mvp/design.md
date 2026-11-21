# Design: Alerting Service MVP

## Context

Repo nie zawiera jeszcze kodu. Motywacja, zakres funkcjonalny i podział na strumienie P1–P4: zob. `proposal.md`. Ograniczenia, które kształtują ten design:

- 18 h, 4 osoby pracujące równolegle z agentami; 3 osoby znają Rusta, 1 osoba robi UI.
- Oceniana jest zmierzona przepustowość i latencja (p50/p99, ingest → dostarczenie), skalowalność pozioma oraz to, czy nowe źródło/schemat/regułę/akcję dodaje się konfiguracją.
- Firma finansowa: bezpieczeństwo wykonywania akcji i przepływ danych do LLM muszą dać się obronić na Q&A.
- „Od świeżego klona do demo” w maks. 2 komendach.

## Goals / Non-Goals

**Goals:**
- Każdy etap przetwarzania to osobny, bezstanowy (lub stan odtwarzalny) konsument Kafki, skalowany przez dołożenie instancji.
- Każdy byt ma **jednego pisarza** (single writer), co eliminuje rozproszone blokady i konflikty między strumieniami.
- Kontrakty (typy, topiki, payloady, API) zamrożone w Fazie 0 tak, by P1–P4 mogły pracować i testować niezależnie.
- Elastyczność przez konfigurację interpretowaną w runtime (mappingi, reguły, szablony, akcje HTTP, runbooki), bez rekompilacji.

**Non-Goals:**
- Exactly-once end-to-end — przyjmujemy at-least-once + idempotencję.
- HA Postgresa i klastra Kafki, multi-tenancy, uwierzytelnianie poza prostym tokenem API.
- Integracja LLM w MVP (tylko interfejs; zob. D12).
- Dokładność licznika dedup przy rebalansie (dopuszczamy utratę kilku inkrementów, nie alertów).

## Decisions

### D1. Rust + jeden Cargo workspace, crate per strumień

```
crates/
  contracts/   Faza 0, wspólny: typy serde, nazwy topików, klucze, kody błędów, fixtures
  condition/   P2: ewaluator warunków + render szablonów (biblioteka; P3 importuje)
  ingest/      P1 bin: HTTP /ingest, konsument źródeł Kafki -> events.raw
  lifecycle/   P1 bin: normalizer + dedup + stan alertów
  router/      P2 bin: reguły + routing -> deliveries.requested
  sender/      P2 bin: adaptery kanałów, retry, fallback, DLQ
  runbooks/    P3 bin: matching, executor, approvals, rejestr akcji
  api/         gateway REST + SSE; moduły routów per strumień (api/src/p1.rs, p2.rs, p3.rs)
  mock-targets/ P3 bin: udawane usługi dla akcji (retry_workflow, restart_service, ticket)
  loadgen/     P1 bin: generator ruchu HTTP + Kafka
ui/            P4: TypeScript + React + Vite
migrations/    1xxx_p1_*.sql, 2xxx_p2_*.sql, 3xxx_p3_*.sql (zakres numerów = właściciel; sqlx wymaga numeru wersji na początku)
config/        sources/, mappings/, rules/, runbooks/, actions/, people.yaml (seed + hot reload)
```

- Wersje zależności tylko w `[workspace.dependencies]` (ustalone w Fazie 0). Strumień dodaje zależność wyłącznie we własnym `Cargo.toml`.
- Stos bibliotek: `tokio`, `axum`, `rdkafka`, `sqlx` (Postgres), `serde`/`serde_json`/`serde_yaml`, `serde_json_path`, `minijinja`, `regex`, `arc-swap`, `notify`, `tracing`, `anyhow`/`thiserror`.
- Jeden obraz Dockera z wieloma binarkami (`cargo-chef` do cache'owania zależności, zwłaszcza `rdkafka`); compose uruchamia ten sam obraz z różnymi komendami.

**Alternatywy:** Go (szybsza kompilacja, porównywalna wydajność) — odrzucone, bo zespół zna Rusta. Mieszanie języków (np. Python dla runbooków) — odrzucone, bo ewaluator warunków i szablony są współdzielone przez P2 i P3.

### D2. Kafka API jako szkielet przetwarzania (lokalnie Redpanda)

| Topik | Klucz | Producent | Konsument (grupa) | Treść |
|---|---|---|---|---|
| `events.raw` | losowy / `source` | ingest (P1) | lifecycle-normalizer (P1) | `Envelope` + `received_at` |
| `events.unmapped` | `schema` | lifecycle (P1) | — (informacyjny) | `Envelope` bez mappingu; trwała kopia w tabeli `unmapped_events`, z której po zapisaniu mappingu zdarzenia są ponownie publikowane na `events.raw` (`FOR UPDATE SKIP LOCKED`, bezpieczne przy wielu instancjach) |
| `events.normalized` | **fingerprint** | lifecycle-normalizer (P1) | lifecycle-dedup (P1) | `CanonicalEvent` |
| `alerts.commands` | **fingerprint** | api/p1 (ack/resolve z UI) | lifecycle-dedup (P1) | `AlertCommand` |
| `alerts.changed` | `alert_id` | lifecycle (P1) | router (P2), runbooks (P3), api (SSE) | `AlertChanged` |
| `deliveries.requested` | `alert_id` | router (P2), runbooks/escalate+notify (P3) | sender (P2) | `DeliveryRequest` |
| `deliveries.retry` | `alert_id` | sender (P2) | sender-retry (P2) | `DeliveryRequest` + `attempt`, `not_before` |
| `deliveries.dlq` | `alert_id` | sender (P2) | api (podgląd), ręczne „wyślij ponownie” | `DeliveryRequest` + ostatni błąd |
| `runbooks.commands` | `alert_id` | api/p3 (approve, done, retry, skip) | runbooks (P3) | `RunbookCommand` |
| `ui.events` | `alert_id` | P1, P2, P3 | api (SSE fan-out) | `UiEvent` (zmiana statusu dostarczenia, kroku itd.) |

- Normalizer i dedup to dwie role tej samej binarki `lifecycle`, rozdzielone topikiem `events.normalized`, bo fingerprint znamy dopiero po normalizacji, a klucz partycji musi nim być (D4).
- Konsument zatwierdza offset **po** zakończeniu pracy i zapisie skutków (at-least-once).
- Jedna wspólna liczba partycji w Fazie 0 (np. 12) — pozwala skalować do 12 instancji na etap bez zmiany topików.
- Producent: `linger.ms` niskie (ścieżka alertu), kompresja `lz4`; parametry w configu, stroimy po pierwszym pomiarze.

**Alternatywy:** wywołania w jednym procesie (niższa latencja, ale utrata zdarzeń w locie i dedup wymagający wspólnego stanu); Kafka tylko na granicach (ten sam problem z partycjonowaniem dedupu); Redis Streams (dodatkowy komponent, a Kafka i tak jest wymaganym źródłem).

### D3. Postgres jako jedyna baza; własność tabel per strumień

| Właściciel | Tabele |
|---|---|
| P1 | `ingest_errors`, `mappings`, `unmapped_events`, `alerts`, `alert_episodes` |
| P2 | `rules`, `people`, `teams`, `team_members`, `preferences`, `deliveries`, `mock_outbox` |
| P3 | `runbooks`, `actions`, `runbook_runs`, `step_runs`, `audit_log`, `unhandled_actions` |

- Tylko właściciel **pisze** do swoich tabel. Inne strumienie mogą je czytać (np. api czyta `alerts`), ale zmiany idą przez topiki `*.commands`.
- Migracje w jednym katalogu, prefiks właściciela, uruchamiane przez `sqlx migrate` przy starcie (kolejność: p1 → p2 → p3; brak kluczy obcych między strumieniami, by nie tworzyć zależności kolejności).
- Redis świadomie pominięty: kolejki, retry i DLQ są w Kafce, stan dedup w pamięci partycji, push do UI przez topik `ui.events`.

### D4. Dedup i cykl życia: pojedynczy pisarz per fingerprint

- `events.normalized` i `alerts.commands` mają klucz = fingerprint, więc wszystkie zdarzenia i polecenia dla jednego F trafiają do jednej instancji `lifecycle`. To ona jest jedynym pisarzem alertu.
- Stan w pamięci: `HashMap<Fingerprint, OpenAlertState { alert_id, episode, count, last_seen, dirty }>`.
- Zdarzenie dla otwartego alertu: `count += 1`, `last_seen = now`, bez publikacji. Co ~1 s zrzut zmienionych liczników do Postgresa jako przyrost (`count = count + delta`, żeby dwie instancje po rebalansie nie nadpisały sobie licznika) i jeden `ui.events` z nowym licznikiem.
- Zmiana stanu (open, ack, resolve, reopen): zapis do Postgresa **synchronicznie** + `alerts.changed`.
- Rebalans: po każdej zmianie przypisania partycji instancja zrzuca zaległe liczniki i czyści cache; stan budowany leniwie — przy chybieniu w cache instancja czyta otwarty alert dla F z Postgresa. Niezrzucone inkrementy z poprzedniej instancji mogą przepaść (licznik może być lekko zaniżony).
- Reopen: zdarzenie dla F, którego ostatni alert jest RESOLVED → ten sam `alert_id`, `episode += 1`, `alerts.changed {reopened}`.

### D5. Hot reload konfiguracji

- Postgres jest źródłem prawdy w runtime. Pliki w `config/` to seed i alternatywny sposób edycji: binarka-właściciel skanuje swój katalog co ~2 s (polling zamiast `notify`, bo bind-mount Dockera na macOS nie przekazuje zdarzeń plikowych) i robi upsert do Postgresa. Seed przy starcie nie nadpisuje zmian zrobionych przez API (`ON CONFLICT DO NOTHING`); upsert następuje dopiero po zmianie pliku.
- Wyjątek: źródła Kafki (`config/p1/sources/*.yaml`) nie mają API ani tabeli — źródłem prawdy są pliki, które `ingest` skanuje i na ich podstawie uruchamia/zatrzymuje konsumentów bez restartu.
- Każda zmiana konfiguracji → `NOTIFY config_changed, '<rodzaj>'`. Konsumenci słuchają `LISTEN`, przeładowują swoją część i podmieniają ją atomowo (`ArcSwap<Config>`). Trwające przetwarzanie dokańcza na starej wersji.
- Każdy zapisany alert/dostarczenie zapamiętuje `rule_version` / `mapping_version`, by było wiadomo, wg której wersji powstał.

### D6. Język warunków i szablony (crate `condition`, właściciel P2)

- Warunek to drzewo serializowane jako JSON/YAML: `{all: [...]}`, `{any: [...]}`, `{not: ...}`, liść `{field, op, value}`. Operatory: `eq, ne, in, gt, gte, lt, lte, contains, regex, exists, glob`.
- `field` to ścieżka po polach kanonicznych (`severity`, `schema`, `source`, ...) lub `raw.<ścieżka>`; brak pola = warunek niespełniony (poza `exists`).
- Warunki kompilowane przy ładowaniu (regex/glob prekompilowane), ewaluacja bez alokacji tam, gdzie się da.
- `explain(cond, event)` zwraca drzewo z wynikiem każdego liścia — na tym opiera się „testuj na zdarzeniu” w UI.
- Szablony: `minijinja` w trybie bez dostępu do plików/funkcji, z limitem rozmiaru wyniku; brakująca zmienna → pusty string + ostrzeżenie w podglądzie.
- Ten sam crate używany przez router (reguły), runbooks (`matches`, parametry akcji) i lifecycle (warunki `severity`/`resolved` w mappingu) — P1 i P3 tylko importują.

### D7. Routing: router rozwiązuje wszystko, sender jest „głupi”

- Router: `alerts.changed {opened|reopened}` → dopasowane reguły → odbiorcy (zespoły rozwinięte na osoby) → dla każdej osoby kanał(y) wg preferencji (severity, godziny; preferencje zespołu jako domyślne) → `DeliveryRequest` z **gotowym łańcuchem fallbacku** i wyrenderowaną treścią per kanał.
- Sender nie zna reguł ani preferencji: wysyła, a przy porażce przechodzi po łańcuchu z wiadomości. Dzięki temu P2 może rozwijać router i sender niezależnie, a nowy kanał to tylko adapter.
- Kilka reguł wskazuje tę samą osobę i kanał → jedno dostarczenie (deduplikacja po kluczu idempotencji, D9).

### D8. Dostarczanie: adaptery, retry, fallback, DLQ

- `trait ChannelAdapter { kind(); send(&Rendered) -> Result<ProviderRef, SendError> }`; `SendError::{Transient, Permanent}`.
- Adaptery MVP: `ui` (zapis + `ui.events`), `email` (SMTP → Mailpit), `sms_mock` i `phone_mock` (zapis do `mock_outbox` + `ui.events`, z przełącznikiem „kanał niedostępny” do demo awarii).
- Porażka transient → `deliveries.retry` z `attempt` i `not_before` (backoff 1 s, 5 s, 30 s). Konsument retry czeka do `not_before` (wstrzymanie partycji). Po wyczerpaniu prób → następny kanał z łańcucha; UI zawsze ostatni; gdy i to zawiedzie → `deliveries.dlq`.
- Porażka permanent (np. zły adres) → od razu następny kanał.
- Status każdego dostarczenia w `deliveries` (`pending, retrying, delivered, failed, fallback, dlq`) + `ui.events`.

### D9. Idempotencja (konsekwencja at-least-once)

| Operacja | Klucz idempotencji | Mechanizm |
|---|---|---|
| zapis zdarzenia w ingest | `event_id` (UUID nadany przy ingest) | — |
| otwarcie/zmiana alertu | `(fingerprint, episode, transition)` | single writer + sprawdzenie stanu |
| dostarczenie | `(alert_id, episode, person_id, channel)` | `UNIQUE` w `deliveries`; ponowna próba wysłania tylko gdy status ≠ `delivered` |
| uruchomienie runbooka | `(alert_id, episode, runbook_id)` | `UNIQUE` w `runbook_runs` |
| wykonanie kroku | `(run_id, step_id, attempt)` | `UNIQUE` w `step_runs`; akcje zewnętrzne dostają nagłówek `Idempotency-Key` |

### D10. Runbooki: wykonanie sekwencyjne jako maszyna stanów w Postgresie

- Runbooks konsumuje `alerts.changed {opened|reopened}`, dopasowuje runbooki (`matches` przez crate `condition`; skrót „przypięty do reguły” = pole `rule_ids` w definicji runbooka: `runbooks` ewaluuje warunki tych reguł z tabeli `rules` tylko do odczytu, bo `AlertChanged` nie niesie `rule_ids` — lifecycle nie zna reguł) i tworzy `runbook_runs` dla każdego dopasowanego. Runbook pasuje, gdy spełniony jest jego `matches` albo pasuje któraś z przypiętych reguł.
- Runbook MANUAL: nie ma wykonania. Dołączenie do powiadomień (link w SMS, pełna treść w email) robi router: sam ewaluuje `matches` runbooków (tabela `runbooks` tylko do odczytu, ten sam crate `condition`), bo router i runbooks konsumują `alerts.changed` równolegle i nie czekają na siebie. Właścicielem definicji runbooków pozostaje P3.
- Runbook AUTOMATIC: kroki wykonywane po kolei. Stan kroku: `pending → running → succeeded | failed | waiting_approval | waiting_manual | skipped`.
  - `waiting_approval` / `waiting_manual` / `failed` zatrzymują sekwencję; wznowienie przez `runbooks.commands` (`approve`, `reject`, `done{note}`, `retry`, `skip`).
  - Każde przejście zapisywane w `step_runs` + `audit_log` (kto, co, kiedy, wynik) + `ui.events`.
- Warunki `if` z runbooka → krok `note` z tekstem warunku.
- Wykonywanie akcji HTTP asynchronicznie z timeoutem; wynik zapisywany przed commitem offsetu/komendy.

### D11. Rejestr akcji i bezpieczeństwo wykonywania

- `trait Action { name(); params_schema(); risk(); execute(ctx, params) }`.
- Akcje kodowe: `escalate` i `notify` (publikują `deliveries.requested` z pominięciem reguł), `http_get`.
- Akcje deklaratywne HTTP z `actions` (YAML/formularz UI): `method`, `url` i `body` jako szablony, `headers` (wartości sekretów jako referencje `${ENV_NAME}`, rozwijane w runtime), `risk`, `timeout_ms`.
- Wymuszenia niezależne od treści runbooka: `risk: high` ⇒ approval; host URL musi być na allowliście (`ACTION_HOST_ALLOWLIST`); brak podążania za przekierowaniami na inne hosty; limit rozmiaru odpowiedzi; nic spoza rejestru się nie wykonuje (nieznana nazwa akcji ⇒ krok `manual` + wpis w `unhandled_actions`).
- `mock-targets` udostępnia endpointy `retry_workflow`, `restart_service`, `open_ticket` z logiem wywołań do pokazania na demo.

### D12. LLM: interfejs z dwoma dostawcami, poza MVP

- `trait RunbookAuthor { draft(description, registry) -> RunbookDraft }` w crate `runbooks`; implementacje: `local` (domyślna, model lokalny np. przez Ollama) i `api` (zewnętrzny dostawca), wybór przez `LLM_PROVIDER=local|api|none`. W MVP tylko trait i `none`.
- Zasady, gdy zostanie zaimplementowane: do LLM trafia wyłącznie opis runbooka (nigdy dane alertów) po redakcji hostów/IP/emaili/URL do tokenów; odpowiedź ograniczona schematem JSON z `action` jako enumem z rejestru; walidacja po stronie backendu; wynik zawsze do akceptacji człowieka; wywołania poza ścieżką alertu.

### D13. API gateway i push do UI

- Jedna binarka `api` (axum): REST + SSE `/stream`. Kod podzielony na moduły per strumień (`p1.rs`, `p2.rs`, `p3.rs`); w `main.rs` lista `merge(...)` zamrożona w Fazie 0.
- Odczyty: bezpośrednio z Postgresa (tabele wszystkich strumieni, tylko odczyt). Zapisy domenowe stanu alertu i runbooka: przez `alerts.commands` / `runbooks.commands` (single writer). Zapisy konfiguracji (reguły, mappingi, akcje): bezpośrednio do tabel właściciela przez jego moduł + `NOTIFY`.
- SSE: `api` konsumuje `alerts.changed` i `ui.events` (grupa per instancja, czyli każda instancja dostaje wszystko, start od końca topiku) i rozsyła przez `tokio::sync::broadcast` do podłączonych klientów, filtrując po zespole (`?team=`); wolny klient gubi zdarzenia (lagged), zamiast blokować innych; strumień kończy się przy zamykaniu `api`. Zrealizowane w `crates/api/src/stream.rs` (sekcja 19.B).
- Kontrakt REST jako OpenAPI w `contracts/openapi.yaml`; P4 do czasu gotowości backendu używa mocka generowanego z tego pliku.
- Uwierzytelnianie: statyczny token API (`API_TOKEN`) wymagany przez `api` dla wszystkich tras poza `/health` — nagłówek `Authorization: Bearer`, a dla SSE także `?token=` (EventSource nie ustawia nagłówków); `ingest` przyjmuje zdarzenia bez tokenu. Do tego wybór „kim jestem” w UI (`X-Person-Id`, bez prawdziwego logowania).

### D14. Pomiar przepustowości i latencji

- Ingest nadaje `event_id` i `received_at`; sender zapisuje `delivered_at` w `deliveries`.
- p50/p99 liczone zapytaniem SQL (`percentile_cont`) po przebiegu testu, mierzone do kanału `ui` lub mocków (nie do Mailpit, by nie mierzyć SMTP).
- `loadgen` pisze jednocześnie na HTTP i na topik źródłowy Kafki, z mieszanką schematów i sterowanym odsetkiem duplikatów (żeby zmierzyć też dedup). Wyniki i skrypt w `docs/LOADTEST.md`.
- Dodatkowo lag konsumentów per topik (`rpk group describe`) jako dowód, gdzie jest wąskie gardło.

### D15. Faza 0: zawartość kontraktów (zamrożona przed rozejściem się)

`crates/contracts` zawiera:
- `Envelope { format, schema, payload, source, event_id, received_at }`
- `CanonicalEvent { event_id, schema, source, severity, title, fingerprint, resolved: bool, fields, raw, received_at, mapping_version }`
- `AlertChanged { alert_id, fingerprint, episode, transition: Opened|Reopened|Acked|Resolved, state, severity, count, event: CanonicalEvent, actor: Option<Actor>, at }` — zmiany samego licznika nie idą na `alerts.changed`, tylko na `ui.events` (`alert_count`), zob. D4
- `AlertCommand { fingerprint, alert_id, kind: Ack|Resolve, actor, at }`
- `DeliveryRequest { delivery_id, alert_id, episode, person_id, chain: Vec<Channel>, rendered: HashMap<Channel, Rendered>, received_at, attempt, not_before }`
- `RunbookCommand { run_id, step_id, kind: Approve|Reject|Done{note}|Retry|Skip, actor, at }`
- `UiEvent { alert_id, kind, payload }`
- `Condition`, `Template` (typy; implementacja w `condition`), `RunbookDef`, `StepDef`, `ActionDef`
- stałe z nazwami topików, funkcje budujące klucze idempotencji, `fixtures/` z trzema zdarzeniami z zadania oraz przykładowymi wiadomościami każdego topiku

Plus: `contracts/openapi.yaml`, `docker-compose.yml` (redpanda, postgres, mailpit, binarki, ui), `Makefile` (`make demo`), puste crate'y z `main.rs`, które się kompilują.

Każdy strumień testuje swój etap niezależnie, publikując fixtures na swój topik wejściowy (`rpk topic produce`) i czytając topik wyjściowy.

## Risks / Trade-offs

- [Czas kompilacji Rusta i budowania `rdkafka` spowalnia pętlę agentów] → `cargo check` w pętli, `cargo-chef` w Dockerze, obraz bazowy zbudowany w Fazie 0, lokalnie `sccache`/`mold`.
- [Każdy przeskok przez Kafkę dodaje kilka ms; ~5 przeskoków podnosi p50] → niski `linger.ms` na ścieżce alertu; w prezentacji pokazujemy p50/p99 obok przepustowości i wyjaśniamy kompromis (trwałość, skalowanie).
- [At-least-once powoduje duplikaty przy restartach] → klucze idempotencji z D9, testy z restartem konsumenta.
- [Rebalans gubi niezrzucone inkrementy licznika dedup] → zrzut co ~1 s, leniwe doczytanie stanu; dokumentujemy ograniczenie.
- [Zmiana kontraktu w trakcie blokuje innych] → zmiany w `contracts/` tylko addytywne (nowe pola `Option`), mały PR + ping; zmiany łamiące tylko za zgodą wszystkich.
- [Konflikty w `api/main.rs` i `Cargo.toml` workspace'u] → lista routów i zależności zamrożona w Fazie 0; każdy edytuje tylko swój moduł/crate.
- [Kafka jako jedno wąskie gardło w compose (jeden broker)] → akceptowalne na demo; partycje pozwalają skalować konsumentów; HA brokera poza zakresem.
- [Akcje HTTP jako wektor SSRF / niezamierzonych zmian w produkcji] → allowlista hostów, wymuszone approvals dla `risk: high`, brak wykonywania czegokolwiek spoza rejestru.
- [Router dodatkowo ewaluuje `matches` runbooków MANUAL, więc zna definicje runbooków P3] → router czyta tabelę `runbooks` tylko do odczytu; format `RunbookDef` jest w `contracts`.

## Migration Plan

Nowy system, brak migracji danych. Start: `docker compose up` (migracje i seed z `config/` przy starcie binarek), opcjonalnie `make demo` (ładuje scenariusz demo i uruchamia generator przykładowych zdarzeń). Rollback = `docker compose down -v`.

## Open Questions

- Eskalacja / ponowne powiadomienie przy braku ack (odłożone funkcjonalnie; jeśli wejdzie, to jako timer w `runbooks` lub `router`).
- Konkretny model lokalny i dostawca API dla `RunbookAuthor` (poza MVP).
- Docelowa liczba partycji i parametry producenta — do ustalenia po pierwszym pomiarze.
- ~~`POST /mappings/preview` wymaga zależności `api -> lifecycle`~~ — rozstrzygnięte: zależność dodana za akceptacją (PR #14), endpoint działa.
- `rule_ids`, `teams`, `color` i prezentacja w `GET /alerts` / `GET /alerts/{id}` wymagają reguł P2 (tabela `rules` + `condition`); lifecycle ich nie zna i `api/p1.rs` nadal zwraca puste wartości. Propozycja: `api/p1.rs` ewaluuje reguły tylko do odczytu przy odczycie alertu (tak jak robi to `runbooks` dla przypiętych reguł) — otwarte, do zrobienia przy integracji (sekcja 18).
- ~~`rule_ids` w `AlertChanged` (D10)~~ — rozstrzygnięte: `rule_ids` jest polem definicji runbooka, a `runbooks` czyta tabelę `rules` tylko do odczytu (PR #17, przegląd 19.A3).
