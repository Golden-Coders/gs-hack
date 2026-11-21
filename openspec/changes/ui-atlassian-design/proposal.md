# Proposal: UI w stylu Atlassian Design System (Jira)

## Why

Konsola operatora (`ui/`) ma własny, ręcznie pisany wygląd: 14 plików CSS, 43 surowe kolory hex w `styles.css`, 7 ręcznie dobranych tonów badge'y i statusy rozróżniane często samym kolorem (czerwona lewa krawędź karty, kolorowy tekst). Dyżurny pracuje w niej pod presją czasu, więc potrzebny jest znany, spójny i dostępny język wizualny (Jira/ADS), obsługa klawiaturą listy alertów i kolejki zatwierdzeń oraz gwarancje w kodzie (lint + testy), że nowe ekrany nie wrócą do surowych kolorów i „gołych” statusów. Teraz, bo refaktor `p4/ui-refactor-1` wprowadził jedną warstwę prymitywów (`ui/src/ui/primitives.tsx`), przez którą migracja może przejść z minimalnymi zmianami w ekranach.

## What Changes

- **Fundament ADS w `ui/`:** pakiety `@atlaskit/*` (tokeny, css-reset, prymitywy układu, przyciski, formularze, tabele, Lozenge, Flag, Modal, SectionMessage, Spinner/Skeleton, EmptyState, Tabs, Tooltip), motyw przez `setGlobalTheme` (jasny/ciemny wg systemu), powłoka aplikacji na `@atlaskit/page-layout` + `@atlaskit/side-navigation`.
- **Ikony z `lucide-react`**, nie z `@atlaskit/icon` (licencja ADG). Bezpośredni import `@atlaskit/icon*` w naszym kodzie zablokowany regułą ESLint.
- **Jedno źródło statusów `ui/src/ui/status.ts`:** każdy status domeny (stan alertu, severity, dostarczenie, krok runbooka, zatwierdzenie, kanał, reguła, połączenie na żywo) → appearance Lozenge + ikona lucide + etykieta PL. Czerwień wyłącznie dla: odrzucone / blokujące / porażka / przekroczony termin. Istniejące `SeverityBadge`, `StateBadge`, `DeliveryStatusBadge`, `StepStatusBadge`, `RiskBadge` zachowują API, a pod spodem używają `status.ts`.
- **Prymitywy (`ui/src/ui/primitives.tsx`) przepisane na ADS z zachowaniem API** (`Card`, `ErrorBox`, `Empty`, `Loading`, `Segmented`, `Tabs`, `Field`, `ConfirmInline`, `CopyButton`) + nowe cienkie prymitywy formularzy (`Select`, `TextInput`, `TextArea`, `Checkbox`), żeby ekrany zmieniały się minimalnie.
- **Gwarancje w kodzie:** `npm run lint` (ESLint 9 + `@atlaskit/eslint-plugin-design-system`, preset `recommended/flat`) oraz test vitest blokujący surowe kolory (`#hex`, `rgb(`, `hsl(`) w `ui/src/**/*.{css,ts,tsx}`. Po fazie 3 lint działa z `--max-warnings 0`.
- **Migracja 9 ekranów operatora + `/dev/components`** w 4 równoległych grupach bez wspólnych plików.
- **UX kolejki zatwierdzeń i feedu alertów (wzorce power-ui na komponentach ADS):** `j`/`k`, `Enter`, `a` zatwierdź, `r` odrzuć (z potwierdzeniem), `x` zaznacz, akcje zbiorcze, paleta komend `⌘K`/`Ctrl+K`, Flag z wynikiem akcji, stany pusto/ładowanie/błąd na każdej liście. Skróty nie działają w polach tekstowych.
- Logika biznesowa, wywołania API, SSE `/stream` (z `?token=`) i łańcuch fallbacku w `Deliveries.tsx` — **bez zmian 1:1**. Kontrakt `contracts/openapi.yaml` — bez zmian.

## Capabilities

### New Capabilities

Brak. Zasady wizualne i narzędzia (tokeny, lint) to szczegół implementacji — opisane w `design.md`.

### Modified Capabilities

- `operator-console`: nowe wymagania widoczne dla operatora — statusy zawsze jako tekst + ikona (nie sam kolor), motyw jasny/ciemny wg systemu, stany pusto/ładowanie/błąd na listach, obsługa klawiaturą feedu i kolejki zatwierdzeń, akcje zbiorcze w kolejce zatwierdzeń, paleta komend, powiadomienie o wyniku decyzji.

## Zależności od aktywnego change'a `unify-and-refactor`

Stan `openspec/changes/unify-and-refactor/tasks.md` na dziś: grupy 1–5 i 6.1 zamknięte (w tym 4.1–4.3 — refaktor `p4/ui-refactor-1` z prymitywami i 147 testami vitest jest w `main`). **Otwarte zadania dotykające UI:**

| Zadanie | Co weryfikuje | Relacja z tym change'em |
|---|---|---|
| 6.2 | kolejka zatwierdzeń na prawdziwym backendzie: zatwierdzenie usuwa krok i aktualizuje szczegóły alertu | ekran `Approvals` przechodzi tu przez fazę 3 i 5 → weryfikację trzeba powtórzyć po migracji |
| 6.3 | mock SMS/telefon z przełącznikiem: wyłączenie SMS pokazuje fallback w szczegółach alertu | `MockOutbox` + `alert-detail/Deliveries` migrowane w fazie 3 |
| 6.4 | edytor reguł: reguła z kreatora bez YAML | `Rules` + `rules/*` migrowane w fazie 3 (największa zmiana formularzy) |
| 6.5 | mappingi i UNMAPPED: nowy schemat → alert w feedzie < 2 min | `Mappings` + `mappings/*` migrowane w fazie 3 |
| 6.6 | formularz akcji HTTP: `flush_cdn` zmienia krok `cdn-stale` na automatyczny | `Actions` + `actions/*` migrowane w fazie 3 |
| 6.7 | README ze świeżego klona | pośrednio: nowe zależności npm muszą instalować się przez `npm ci` w kontenerze `ui` |
| 6.8 | 2 próby generalne `docs/DEMO.md` + nagranie ≤ 5 min | nagranie powinno pokazywać nowe UI → **6.8 po zakończeniu tego change'a** |

Wnioski:
- **Ten change nie jest zablokowany** przez 6.2–6.8 — jego warunek wstępny (4.1–4.3) jest spełniony.
- 6.2–6.6 wykonane teraz na starym UI trzeba by powtórzyć. Zamiast tego faza 4 tego change'a (zadanie 4.4) przechodzi scenariusze 6.2–6.6 na prawdziwym backendzie po migracji; wynik pozwala odhaczyć 6.2–6.6 w `unify-and-refactor`. Jeśli 6.2–6.6 zostaną odhaczone wcześniej (stare UI), zadanie 4.4 i tak je powtarza jako regresję.
- 6.8 (nagranie demo) zaplanować po fazie 5; 6.7 jest niezależne, ale lepiej wykonać je po fazie 2 (nowe zależności).
- Kryteria przeglądu z `unify-and-refactor/specs/code-review` (poprawność, bezpieczeństwo, wydajność, minimalność) obowiązują też fazę 4 tego change'a.

## Impact

- **Kod:** `ui/package.json`, `ui/package-lock.json`, nowy `ui/eslint.config.js`, `ui/vitest.config.ts` (bez zmian środowiska), `ui/src/main.tsx`, `ui/src/App.tsx`, `ui/src/styles.css`, `ui/src/ui/*` (w tym nowe `status.ts`, `keyboard.ts`), `ui/src/components/*`, wszystkie `ui/src/screens/**`. Pliki `.css` ekranów kurczą się lub znikają.
- **React 19 → 18.3.1** (`react`, `react-dom` 18.3.1, `@types/react` 18.3.31, `@types/react-dom` 18.3.7): przechodnie zależności ADS (`@atlaskit/portal`, `analytics-next`, `interaction-context`, `browser-apis`) mają peer tylko `react ^18.2.0` — szczegóły w `design.md` D1.
- **Zależności (nowe, bezpośrednie z peer `react ^18.2.0 || ^19.0.0`, wersje dokładne):** `@atlaskit/tokens`, `@atlaskit/css-reset`, `@atlaskit/primitives`, `@atlaskit/button`, `@atlaskit/lozenge`, `@atlaskit/badge`, `@atlaskit/heading`, `@atlaskit/form`, `@atlaskit/textfield`, `@atlaskit/textarea`, `@atlaskit/select`, `@atlaskit/checkbox`, `@atlaskit/radio`, `@atlaskit/toggle`, `@atlaskit/tabs`, `@atlaskit/dynamic-table`, `@atlaskit/flag`, `@atlaskit/modal-dialog`, `@atlaskit/section-message`, `@atlaskit/spinner`, `@atlaskit/skeleton`, `@atlaskit/empty-state`, `@atlaskit/tooltip`, `@atlaskit/code`, `@atlaskit/link`, `@atlaskit/menu`, `@atlaskit/page-header`, `@atlaskit/page-layout`, `@atlaskit/side-navigation`, `lucide-react`; dev: `eslint@^9` (najnowsze 10.x nie jest jeszcze sprawdzone z pluginem), `typescript-eslint`, `@atlaskit/eslint-plugin-design-system`. **Nie** używamy `@atlaskit/navigation-system` (peer tylko `react ^18.2.0`).
- **Rozmiar bundla** wzrośnie (ADS ciągnie `@emotion/react`, `@compiled/react`, `react-select`, przechodnio `@atlaskit/icon`) — mierzony przed i po, z lazy-loadingiem ekranów konfiguracji, jeśli główny chunk przekroczy próg ostrzeżenia Vite.
- **Bez zmian:** backend Rust, `contracts/openapi.yaml`, `ui/src/api/*` (poza ewentualnym czystym helperem testowym), Docker Compose (kontener `ui` robi `npm ci` przy starcie).
- **Agenci:** fazy realizują agenci z `.claude/agents/` (`ads-foundation`, `screen-migrator`, `test-writer`, `design-guardian`, `code-reviewer`, `approval-ux`) — przypisanie w `tasks.md`.
