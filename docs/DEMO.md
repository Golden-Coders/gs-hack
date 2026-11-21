# Scenariusz demo (nagranie ≤ 5 min)

Start: `git clone ... && make demo` (2 komendy; `API_PORT=8088 make demo`, gdy 8080 zajęty).
`make demo` buduje stack, czeka na health i uruchamia generator `scripts/demo/run.sh`
(powtarzalny; sam generator: `make demo-events`; wyłączenie kroku awarii SMS: `DEMO_OUTAGE=0`).

Adresy: UI http://localhost:5173, Mailpit http://localhost:8025, mock-targets http://localhost:8090/calls.

## Plan nagrania

| Czas | Krok (komenda / klik) | Oczekiwany efekt |
|---|---|---|
| 0:00 | Slajd/narracja: architektura (README), `make demo` | stack wstaje, generator wypisuje podsumowanie |
| 0:30 | UI → Alerty | alerty z 3 schematów Appendix A (workflow, alertmanager, trade); kolory/pola z reguł |
| 1:00 | Dedup: skrypt (b) wysłał ×500 `WORKFLOW_FAILED` | jeden alert z licznikiem ≈ 500, nie 500 alertów; jedno powiadomienie |
| 1:20 | Routing wg preferencji: alert `HighErrorRate` (payments, critical) do Anny | w ciągu 8:00–20:00 SMS (Skrzynka mocka), poza godzinami e-mail (Mailpit); zespół Settlements dostaje wg preferencji zespołu |
| 1:50 | Awaria SMS: `PUT /mock/channels/sms {"available":false}` (krok (d) skryptu robi to sam) + nowy alert | dostarczenie SMS = błąd techniczny → fallback na e-mail/UI; w UI status dostarczenia; potem kanał wraca |
| 2:20 | Runbook AUTOMATIC: alert Settlements (krok (c)) | kroki: check zdrowia → retry → eskalacja z `approval: required`; wstrzymanie na zatwierdzeniu; klik „Zatwierdź” w UI (tylko z UI), dziennik kroków |
| 2:50 | Runbook MANUAL | treść runbooka w e-mailu (payments, Mailpit) i link w SMS (pricing, skrzynka mocka) |
| 3:10 | Nowe źródło konfiguracją: UI → Nieznane (UNMAPPED) → `judges.custom.v1` (krok (e)) → „Dodaj mapping” z podglądem | zdarzenie wraca do przetworzenia i powstaje alert (< 2 min) |
| 3:50 | Nowa akcja HTTP formularzem (UI → Akcje → dodaj, np. `flush_cdn` na `mock-targets`) | akcja w rejestrze w < 1 min; runbook z wcześniej nieznaną akcją przestaje być krokiem manualnym, licznik brakujących akcji maleje |
| 4:15 | Zmiana reguły bez restartu (UI → Reguły: zmień severity/odbiorcę, zapisz) i wyślij zdarzenie (`make demo-events`) | nowe zachowanie od razu (hot reload) |
| 4:35 | Bezpieczeństwo: runbook z krokiem `shell: rm -rf /` | krok manualny, nic się nie wykonuje; akcja na host spoza `ACTION_HOST_ALLOWLIST` odrzucona; `risk: high` wymaga zatwierdzenia |
| 4:50 | Wydajność: wyniki z `docs/LOADTEST.md` | przepustowość, p50/p99, 2 instancje lifecycle/sender |

## Krok (f): runbook z nieznaną akcją

Runbooki ładowane są wyłącznie z `config/runbooks/` (strumień P3), więc generator demo
nie dodaje tam pliku. Przed nagraniem dodaj ręcznie (seed P3, hot reload) runbook
YAML zawierający krok z `action: flush_cdn` (akcji nie ma w `config/actions`) i dopasowanie
np. do `trade.rejected`. Efekt: krok manualny + licznik brakujących akcji w UI. Akcję dodaj
w kroku 3:50, by pokazać „< 1 min”.

## Próba generalna (checklista)

Wykonaj 2× na czystym klonie, bez ręcznych poprawek (`make down` między przebiegami):

- [ ] `make demo` kończy się kodem 0 i wypisuje podsumowanie (alert z licznikiem ≥ 500, UNMAPPED zawiera `judges.custom.v1`)
- [ ] UI na :5173 pokazuje alerty, dostarczenia i runbook oczekujący na zatwierdzenie
- [ ] Skrzynka mocka (`/mock/outbox`) zawiera SMS/telefon, Mailpit — e-maile
- [ ] Awaria SMS: dostarczenie przechodzi fallbackiem, po skrypcie SMS jest znów dostępny (`GET /mock/channels`)
- [ ] Mapping dla UNMAPPED dodany z UI, alert powstaje < 2 min
- [ ] Akcja HTTP dodana z UI < 1 min i użyta w runbooku
- [ ] Zmiana reguły działa bez restartu
- [ ] Runbook z `shell` niczego nie wykonuje
- [ ] `make demo` powtórzony na działającym stacku nie zgłasza błędów
- [ ] Nagranie mieści się w 5 min (tekst narracji ≈ 600 słów)
