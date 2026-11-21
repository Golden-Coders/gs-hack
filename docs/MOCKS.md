# Co jest symulowane i jak to podmienić

Zadanie pozwala symulować kanały zewnętrzne. Oceniane jest to, czy podmiana na prawdziwego dostawcę to mała, odizolowana zmiana. Poniżej każdy element `docker-compose.yml`, który nie jest „prawdziwy”, oraz miejsca w UI i API, które istnieją tylko na potrzeby demo.

## Podsumowanie

| Element | Co udaje | W compose | Podmiana |
|---|---|---|---|
| SMS (`sms_mock`) | bramkę SMS | adapter w `sender`, zapis do `mock_outbox` | nowy adapter `ChannelAdapter` |
| Telefon (`phone_mock`) | bramkę połączeń głosowych | adapter w `sender`, zapis do `mock_outbox` | nowy adapter `ChannelAdapter` |
| Mailpit | serwer SMTP | usługa `mailpit` (:1025 SMTP, :8025 podgląd) | zmienne `SMTP_HOST` / `SMTP_PORT` |
| `mock-targets` | usługi, na których działają akcje runbooków | usługa `mock-targets` (:8090) | URL w definicji akcji + `ACTION_HOST_ALLOWLIST` |
| Uwierzytelnianie | logowanie użytkowników | `API_TOKEN=dev-token` + nagłówek `X-Person-Id` | middleware OIDC w `api` |
| Redpanda | klaster Kafki | usługa `redpanda` | `KAFKA_BROKERS` |
| Prism | backend dla UI w trakcie developmentu | poza compose (`npm run mock`) | przełącznik w UI |
| LLM (`RunbookAuthor`) | autor runbooków z opisu | brak (implementacja `none`) | implementacja traitu |

Nic z tej listy nie wymaga zmian w regułach, mappingach, runbookach ani w UI.

## Kanały SMS i telefon

<!-- nazwy plików i tabel do potwierdzenia przez P2 po implementacji sendera -->
**Co robią.** Zamiast wysyłać, adaptery `sms_mock` i `phone_mock` w `crates/sender` zapisują wiadomość do tabeli `mock_outbox` i publikują `ui.events`. W UI widać je na ekranie **SMS / telefony**, z treścią, adresatem i linkiem do alertu. Nieodebrany telefon nie jest porażką dostarczenia; porażką jest tylko błąd techniczny kanału.

**Symulacja awarii.** Na tym samym ekranie można wyłączyć kanał (`PUT /mock/channels/{channel}` z `{"available": false}`). Kolejne wysyłki zwracają wtedy błąd techniczny. Sender ponawia je (1 s, 5 s, 30 s), potem przechodzi do następnego kanału z łańcucha osoby (UI jest zawsze ostatni). W szczegółach alertu widać próby i status „kanał zapasowy”. Gdy zawiodą wszystkie kanały, wiadomość trafia na `deliveries.dlq` i do zakładki „Nieudane” z przyciskiem „Wyślij ponownie”.

**Podmiana na prawdziwego dostawcę** (np. Twilio, SMSAPI, Vonage):
1. Nowy adapter w `crates/sender` implementujący `ChannelAdapter`: `kind()` zwraca `sms` albo `phone`, a `send(&Rendered)` wywołuje API dostawcy. Błędy sieci i odpowiedzi 5xx/429 mapujemy na `SendError::Transient`, zły numer na `SendError::Permanent`.
2. Klucz API przez zmienną środowiskową binarki `sender`, nigdy w pliku.
3. Rejestracja adaptera zamiast mocka dla danego kanału.

Retry, fallback, DLQ, idempotencja (`(alert, epizod, osoba, kanał)`) i statusy działają w senderze wspólnie dla wszystkich kanałów, więc nie trzeba ich ruszać. Endpointy `/mock/*` i ekran **SMS / telefony** przestają być potrzebne.

## E-mail: Mailpit

**Co robi.** Adapter `email` w `sender` wysyła prawdziwe SMTP, tyle że do Mailpit, który niczego nie dostarcza dalej. Wiadomości z pełną treścią alertu i runbookiem są widoczne na http://localhost:8025.

**Podmiana.** Ustaw `SMTP_HOST` i `SMTP_PORT` serwisu `sender` na prawdziwy serwer (np. relay firmowy albo Amazon SES SMTP) i dodaj dane logowania przez zmienne środowiskowe. <!-- do potwierdzenia przez P2: TLS i uwierzytelnianie SMTP --> Kod adaptera się nie zmienia.

## Akcje runbooków: mock-targets

**Co robi.** `crates/mock-targets` (:8090) udaje usługi, na których runbooki wykonują akcje: ponowienie workflow, restart usługi, zgłoszenie (ticket), czyszczenie cache'u. Loguje każde wywołanie (z nagłówkiem `Idempotency-Key`), żeby na demo było widać, że akcja naprawdę poszła. Może też zwrócić błąd, żeby pokazać krok „błąd” i przyciski „ponów” / „pomiń” / „tryb ręczny”. <!-- lista endpointów do potwierdzenia przez P3 -->

**Podmiana.** W definicji akcji (`config/actions/*.yaml` albo ekran **Akcje**) zmień `url` na prawdziwą usługę, a jej host dopisz do `ACTION_HOST_ALLOWLIST`. Sekrety podaj jako `${ENV_NAME}` i ustaw zmienną w serwisie `runbooks`. Akcje kodowe (`escalate`, `notify`) nie korzystają z mock-targets.

## Uwierzytelnianie i tożsamość

**Co robi.** API przyjmuje jeden statyczny token (`API_TOKEN`, domyślnie `dev-token`). Kim jest użytkownik, wybiera się w UI („Działam jako”). UI wysyła to jako nagłówek `X-Person-Id`, który trafia do dziennika jako autor ack, resolve, zatwierdzeń i komend kroków. Ten sam wybór steruje filtrem „mój zespół” w feedzie.

**Podmiana.** Middleware w `crates/api`, który weryfikuje token OIDC (np. Entra ID, Okta) i bierze `person_id` z claimów zamiast z nagłówka. W UI logowanie zastępuje wybór osoby. Autoryzację (kto może zatwierdzać akcje `risk: high`, kto edytuje reguły zespołu) dodaje się w tym samym miejscu.

## Kafka: Redpanda

To nie jest mock, tylko zgodna z Kafka API implementacja, lżejsza do uruchomienia lokalnie. Binarki używają `rdkafka`, więc działają z każdym klastrem Kafki (MSK, Confluent, Strimzi).

**Podmiana.** `KAFKA_BROKERS` (i ewentualnie parametry SASL/TLS w `crates/common`) oraz utworzenie topików z `contracts::topics::ALL` z liczbą partycji `contracts::topics::PARTITIONS`. W compose robi to usługa `topics-init`.

## Prism: mock API dla UI

Używany tylko w trakcie developmentu UI (`cd ui && npm run mock`, :4010), poza compose. Odpowiada przykładami z `contracts/openapi.yaml`, więc UI dało się budować przed gotowym backendem. W UI przełącza się go w panelu bocznym („Backend: mock / prawdziwy”). Prism nie obsługuje SSE, więc na mocku nie ma aktualizacji na żywo; UI mówi to wprost. W docelowym uruchomieniu nie bierze udziału.

## LLM

W MVP nie ma wywołań modeli. W `crates/runbooks` jest interfejs `RunbookAuthor` z implementacją `none` (`LLM_PROVIDER=none`). Docelowo implementacje `local` (np. Ollama) i `api`. Zasady prywatności dla nich opisuje README, sekcja „Użycie AI”.
