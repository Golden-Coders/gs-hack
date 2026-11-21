# Scenariusz nagrania: co klikać, co mówić

Uzupełnia `docs/PLAN-NAGRANIA.md` (czasy i kryteria). Tu: ekran po ekranie. Każda scena to osobne ujęcie.
Nazwy przycisków pochodzą z kodu UI (`ui/src/screens`); przy próbie generalnej zweryfikuj je na ekranie.

Okna przygotowane przed nagraniem (osobne karty): **UI** http://localhost:5173, **Mailpit** http://localhost:8025,
**mock-targets** http://localhost:8090/calls, terminal z dużą czcionką w katalogu repo.

---

## Scena 1 (0:00–0:20): problem
Slajd tytułowy. Bez klikania.
> „Alerty giną w logach albo jest ich tyle, że nikt ich nie czyta. Zbudowaliśmy serwis, który dostarcza alert właściwej osobie, na jej kanale, i prowadzi ją przez runbook do rozwiązania.”

## Scena 2 (0:20–0:45): architektura i start
1. Slajd z diagramem (`docs/diagram.md`).
2. Terminal: `docker compose down -v` (przed nagraniem), potem w ujęciu: `make demo`. Przyspiesz ×8 z podpisem „×8”.
3. Zatrzymaj kadr na podsumowaniu generatora (alert z licznikiem ≥ 500, UNMAPPED zawiera `judges.custom.v1`).
> „Dwie komendy od klona. Kafka w środku, bezstanowe serwisy skalowane poziomo, Postgres jako źródło prawdy.”

## Scena 3 (0:45–1:15): trzy schematy i dedup
1. Karta UI → menu **Alerty**.
2. Wskaż po kolei trzy alerty: workflow (`WORKFLOW_FAILED`), alertmanager (`HighErrorRate`), trade (`trade.rejected`). Pokaż, że mają różne kolory i pola (to z reguł).
3. Wskaż alert `WORKFLOW_FAILED` z licznikiem ≈ 500. Kliknij go → szczegóły alertu, pokaż licznik i oś zdarzeń.
> „Trzy różne schematy, dwa źródła: HTTP i Kafka. Zero kodu, tylko mappingi. Pięćset identycznych błędów to jeden alert z licznikiem i jedno powiadomienie.”

## Scena 4 (1:15–1:45): routing wg preferencji
1. UI → szczegóły alertu `HighErrorRate` → sekcja **Dostarczenia** (kanał, status).
2. Menu **SMS / telefony** → pokaż SMS dla Anny (mock).
3. Karta Mailpit → pokaż e-mail dla zespołu Settlements.
4. Menu **Osoby** → pokaż preferencje Anny (SMS 8–20, poza tym e-mail).
> „Anna dostaje SMS w godzinach pracy, a poza nimi e-mail. Zespół ma własne preferencje.”

Uwaga: nagrywasz po 20:00, to w Mailpit zamiast SMS zobaczysz e-mail. Powiedz to wprost.

## Scena 5 (1:45–2:05): awaria kanału
Ujęcie przygotuj wcześniej: `scripts/demo/run.sh` krok (d) sam wyłącza SMS i wysyła alert.
1. UI → alert z regionem `demo-…` → **Dostarczenia**: próba SMS = błąd, potem fallback na e-mail/UI.
2. Opcjonalnie terminal: `curl -s localhost:8090/…` nie jest potrzebne; wystarczy widok dostarczeń.
> „SMS padł, alert nie zginął. Retry z backoffem, fallback na e-mail, a na końcu DLQ. Offset Kafki commitujemy dopiero po zapisaniu skutków.”

## Scena 6 (2:05–2:45): runbook z zatwierdzeniem
1. UI → **Alerty** → alert Settlements `demo-step-…` → zakładka/sekcja z runbookiem.
2. Pokaż kroki: health check ✓, retry ✓, eskalacja **czeka na zatwierdzenie**.
3. Menu **Zatwierdzenia** → kolejka z krokiem eskalacji.
4. Kliknij **Zatwierdź**. Wróć do alertu i pokaż dziennik kroków (kto, co, kiedy).
> „Runbook z Appendix B wykonuje się sam, aż potrzebny jest człowiek. Ryzykowne kroki zawsze wymagają zatwierdzenia i zostawiają ślad.”

Pokaż też przycisk **Odrzuć** i że odrzucenie wymaga powodu.

## Scena 7 (2:45–3:00): runbooki ręczne
1. Mailpit → e-mail payments: pełny Markdown runbooka z podstawionymi wartościami.
2. UI → **SMS / telefony** → SMS pricing: krótki link do runbooka.
> „Markdown, YAML i wolny tekst. Pełne kroki w e-mailu i UI, w SMS tylko link.”

## Scena 8 (3:00–3:35): nieznany schemat (test jury)
1. UI → **Mappingi** → lista nieznanych (UNMAPPED) → `judges.custom.v1`.
2. Otwórz przykładowe zdarzenie, wskaż pola (`host`, `level`, `msg`).
3. Kliknij **Utwórz mapping**, wskaż pola na tytuł/severity, pokaż **podgląd**, zapisz (**Zapisz zmiany**).
4. Wróć do **Alerty**: pojawia się alert z `judges.custom.v1` (zdarzenie przetworzone ponownie).
> „Schemat od jury nie ginie. Trafia do nieznanych, klikamy mapping z podglądem i zdarzenie wraca do przetwarzania.”

## Scena 9 (3:35–3:55): nowa akcja bez deployu
Przed ujęciem: w UI → **Akcje** nie może być `flush_cdn`. Wyślij alert `CDN_STALE` komendą z `PLAN-NAGRANIA.md` (Przygotowanie, pkt 3).
1. UI → alert `CDN_STALE` → krok `flush_cdn` jako **ręczny**.
2. Menu **Akcje** → karta **Brakujące akcje z runbooków**: widać `flush_cdn`.
3. **Nowa akcja** → nazwa `flush_cdn`, rodzaj HTTP, `POST http://mock-targets:8090/flush_cdn`, ryzyko low → **Utwórz akcję**.
4. Poczekaj ok. 2 s (przeładowanie rejestru), wyślij drugi `CDN_STALE`.
5. Nowy alert: krok wykonany automatycznie. Karta mock-targets `/calls` pokazuje wywołanie.
> „Nieznana akcja degraduje się do kroku ręcznego zamiast psuć runbook. Dodajemy ją formularzem i działa w kilkanaście sekund.”

## Scena 10 (3:55–4:10): hot reload reguły
1. UI → **Reguły** → wybierz regułę (np. Settlements) → zmień severity lub odbiorcę → **Zapisz regułę**.
2. Terminal: `make demo-events`.
3. UI → **Alerty**: nowy alert ma zmienione zachowanie.
> „Zmiana reguły bez restartu. Konfiguracja leci przez Postgres i `NOTIFY`.”

## Scena 11 (4:10–4:20): bezpieczeństwo
1. UI → **Runbooki** → `shell-demo`: krok `shell: rm -rf /` oznaczony jako ręczny, nic się nie wykonało.
2. Wspomnij (slajd lub głos): allowlista hostów, `risk: high` zawsze z zatwierdzeniem, sekrety tylko jako `${ENV}`.

## Scena 12 (4:20–4:40): wydajność
Slajd z tabelą z `docs/PLAN-NAGRANIA.md` (przed/po). W tle mignij `loadtest/step_load.py` i katalog `loadtest/results/`.
> „Zmierzone, nie oszacowane. Skrypt i surowe wyniki są w repo.”

## Sceny 13–14 (4:40–5:00): AI i zakończenie
Slajd „jak używaliśmy AI” (OpenSpec, testy, review, dane syntetyczne, sekrety jako `${ENV}`), potem slajd z linkiem do repo i `docs/MOCKS.md`.

---

## Kolejność nagrywania (oszczędza czas)
Nie nagrywaj w kolejności filmu. Jeden `make demo`, potem:
1. Sceny 3, 4, 6, 7 (stan po `make demo`).
2. Scena 5 (wymaga świeżego kroku (d); powtórz `make demo-events`).
3. Sceny 8, 9, 10 (zmieniają konfigurację; po każdej cofnij zmianę).
4. Scena 2 na końcu, bo wymaga `down -v` i ponownego startu.
5. Sceny 12–14: slajdy.

## Powtórki
`make demo-events` jest powtarzalne. Przy scenie 9 usuń `flush_cdn` w UI → **Akcje**, żeby wrócić do stanu wyjściowego.
