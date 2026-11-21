# Design: UI w stylu Atlassian Design System (Jira)

## Context

Motywacja: `proposal.md` → Why. Stan obecny `ui/` (gałąź `main` po refaktorze `p4/ui-refactor-1`):

- React 19 (w fazie 2 zejście na React 18.3 — D1), Vite 6, TypeScript 5.6 (`skipLibCheck: true`), react-router 7, vitest 3 w środowisku `node` (tylko `src/**/*.test.ts`, bez DOM), npm (`package-lock.json`), kontener `ui` robi `npm ci` przy starcie.
- Wygląd: własne zmienne CSS w `styles.css` (43 kolory hex, w tym ciemny pasek nawigacji), style prymitywów w `ui/ui.css`, 9 plików CSS ekranów i 3 komponentów. Statusy jako `Badge` z 7 tonami (`components/Badges.tsx`), w kilku miejscach wyróżniane samym kolorem (lewa krawędź `.ad-step-*`, `.approval-high`, `.channel-down`, `.rules-saved`).
- Nie ma ESLinta ani żadnej automatycznej kontroli kolorów.
- Źródła dla decyzji: lustro dokumentacji ADS (`~/.claude/skills/atlassian-design/references/docs/`), `npm view <pkg> peerDependencies` (wyniki niżej), typy pakietów `@atlaskit/lozenge`, `@atlaskit/tokens`, `@atlaskit/side-navigation`, `@atlaskit/page-layout`, `@atlaskit/eslint-plugin-design-system` i `lucide-react` pobrane przez `npm pack` (bez instalacji w repo).

### Inwentaryzacja: ekrany (`ui/src/screens`)

Trasy z `App.tsx`: 9 ekranów operatora + 1 deweloperski (`/dev/components`).

| # | Ekran (trasa) | Pliki | Rola | Prymitywy / komponenty | Natywne elementy do zastąpienia | CSS (linie; użycia `--red/…/--gray`) | Inline style / hex |
|---|---|---|---|---|---|---|---|
| 1 | Alerty (`/`) | `AlertFeed.tsx` (142) | feed na żywo, filtr „mój zespół / wszystkie”, flash zmian | `Segmented`, `SeverityBadge`, `StateBadge`, `paletteColor` | `<table>`, `<button>`, `<h1>`, `<p class="error-box">` | `AlertFeed.css` (21; 1: `--yellow-bg` w animacji flash) | `boxShadow` z koloru reguły |
| 2 | Szczegóły alertu (`/alerts/:id`) | `AlertDetail.tsx` (203), `alert-detail/{Audit,Deliveries,Episodes,Runbooks}.tsx` (42/147/41/154) | prezentacja wg reguły, ack/resolve, runbooki z komendami kroków, dostarczenia z łańcuchem fallbacku i „wyślij ponownie”, epizody, dziennik | `Card`, `ErrorBox`, `Loading`, `Tabs`, `Badge`, `StepStatusBadge`, `DeliveryStatusBadge`, `Presentation`, `Markdown` | 4× `<table>`, 7× `<button>`, `<input>` notatki, `<dl>` | `AlertDetail.css` (38; 8: krawędzie kroków wg statusu, `.ad-error-text`) | — |
| 3 | Zatwierdzenia (`/approvals`) | `Approvals.tsx` (201) | kolejka kroków `waiting_approval`, zatwierdź / odrzuć (z potwierdzeniem w miejscu), optymistyczne ukrycie 15 s | `ConfirmInline`, `Empty`, `ErrorBox`, `Loading`, `RiskBadge` | `<ul>` kart, `<button>`, `<dl>` | `Approvals.css` (13; 2: `.approval-high` czerwona krawędź) | — |
| 4 | SMS / telefony (`/mock-outbox`) | `MockOutbox.tsx` (174) | wysłane wiadomości z symulowanych kanałów, przełącznik awarii kanału | `Card`, `Empty`, `ErrorBox`, `Badge` | `<table>`, `<button>` przełącznika | `MockOutbox.css` (8; 4: zielona/czerwona karta kanału) | — |
| 5 | Reguły (`/rules`) | `Rules.tsx` (141), `rules/{RuleEditor,ConditionBuilder,RuleTester}.tsx` (596/218/209), `rules/model.ts` | lista reguł + edytor (kreator ↔ YAML, odbiorcy, prezentacja, test na zdarzeniu), zapis/usuwanie | `Card`, `Field` (16×), `Tabs`, `Segmented`, `ConfirmInline`, `ErrorBox`, `Empty`, `Loading`, `Badge`, `paletteColor`, `formatValue` | 10× `<button>`, 4× `<input>`, 4× `<select>`, 2× `<textarea>`, checkboxy | `Rules.css` (95; 16) | swatch koloru reguły |
| 6 | Mappingi (`/mappings`) | `Mappings.tsx` (249), `mappings/{MappingEditor,PreviewPanel}.tsx` (628/128), `mappings/{draft,localPreview,sample}.ts` | lista UNMAPPED z przykładowym payloadem, edytor mappingu (wybór pól z drzewa), podgląd | `Card`, `Field` (15×), `Segmented`, `ConfirmInline`, `ErrorBox`, `Empty`, `Loading`, `SeverityBadge`, `PayloadTree`, `ConditionView`, `formatValue` | 16× `<button>`, 2× `<select>`, radio, `<details>`, `<pre>`, `<table>` | `Mappings.css` (68; 4) | — |
| 7 | Akcje (`/actions`) | `Actions.tsx` (261), `actions/ActionForm.tsx` (285), `actions/model.ts` | rejestr akcji, brakujące akcje z „Dodaj”, formularz akcji HTTP | `Card`, `Field` (8×), `ConfirmInline`, `ErrorBox`, `Empty`, `Loading`, `RiskBadge` | 2× `<table>`, 5× `<button>`, 2× `<input>`, 2× `<select>`, `<textarea>`, 7× `<code>` | `Actions.css` (36; 4) | — |
| 8 | Runbooki (`/runbooks`, tylko odczyt) | `Runbooks.tsx` (150) | lista runbooków z plików, warunek, treść MANUAL / kroki AUTOMATIC | `Badge` (4×), `Markdown`, `ConditionView`, `SourceFile`, `ReadOnlyNote`, `Empty`, `ErrorBox`, `Loading` | `<details>`, `<pre>`, `<ol>` | `Runbooks.css` (22; 0) | — |
| 9 | Osoby (`/people`, tylko odczyt) | `People.tsx` (190) | osoby, zespoły, preferencje kanałów wg severity, ścieżka pliku | `Card`, `SeverityBadge`, `SourceFile`, `ReadOnlyNote`, `Empty`, `ErrorBox`, `Loading` | `<table>`, `<ul>` | `People.css` (18; 0) | — |
| 10 | Komponenty (`/dev/components`, poza menu) | `DevComponents.tsx` (83) | podgląd prymitywów i badge'y na danych z API | wszystkie prymitywy i badge'e | `<button>`, `<input>` | — | — |

### Inwentaryzacja: komponenty (`ui/src/components`)

| Komponent | Rola | Używany w | CSS | Uwagi |
|---|---|---|---|---|
| `Badges.tsx` | `Badge`, `SeverityBadge`, `StateBadge`, `DeliveryStatusBadge`, `StepStatusBadge`, `RiskBadge` (7 tonów) | 9 plików | `styles.css` `.badge-*` | → delegacja do `status.ts`, API bez zmian |
| `colors.ts` (+ test) | nazwa koloru z reguły → `var(--red)` / `var(--red-bg)` | `AlertFeed`, `Presentation`, `RuleEditor` | — | API i test bez zmian; zmienne `--red…--gray` stają się aliasami tokenów akcentowych ADS (D4) |
| `Presentation.tsx` | alert wg prezentacji reguły, `PresentedFields`, `formatValue` | `AlertDetail`, `DevComponents`, `RuleTester`, `PreviewPanel` | `styles.css` `.presentation*` (hex `#fff`, `#e5e7eb`, `#9ca3af`, `#f9fafb`) | kolory z reguły zostają (wyjątek od reguły czerwieni) |
| `Markdown.tsx` | runbooki MANUAL (react-markdown) | `alert-detail/Runbooks`, `Runbooks`, `DevComponents` | `styles.css` `.markdown` (hex) | tylko tokeny |
| `ConditionView.tsx` | drzewo warunków (odczyt) | `Runbooks`, `MappingEditor` | `ConditionView.css` (10; 1: `.not` czerwony) | „nie” to operator, nie porażka → bez czerwieni |
| `PayloadTree.tsx` | drzewo payloadu z wyborem pól | `MappingEditor` | `PayloadTree.css` (20; 5: kolory typów wartości) | kolory typów → tokeny tekstu akcentowego |
| `SourceFile.tsx` | ścieżka pliku + kopiuj, `ReadOnlyNote` | `People`, `Runbooks` | `SourceFile.css` (8) | `Code` + `CopyButton` |
| `SessionBar.tsx` | „działam jako”, przełącznik backendu | `App` (nav) | `styles.css` `nav .session*` (hex) | → stopka nawigacji |
| `StreamIndicator.tsx` | stan SSE + trzymanie subskrypcji | `SessionBar` | `styles.css` `.stream-*` (hex) | → `status.ts` (domena `stream`) |

### Inwentaryzacja: prymitywy (`ui/src/ui/primitives.tsx`, style `ui/ui.css`)

| Prymityw | Użycia (pliki) | Obecnie | Docelowo (D5) |
|---|---|---|---|
| `Card` | 13 | `<section class="card">` + nagłówek, „ładowanie…”, `ErrorBox` | `Box` z tokenami tła/obramowania + `Heading`; ładowanie = `Spinner` w nagłówku |
| `ErrorBox` | 14 | `div.error-box role=alert` + przycisk ponów | `SectionMessage appearance="error"` z akcją „Spróbuj ponownie” |
| `Empty` | 9 | `p.empty` z przerywaną ramką | tekst `Text color="color.text.subtlest"` w ramce z tokenów; pełnoekranowe puste stany → `EmptyState` |
| `Loading` | 8 | `p.loading` | `Spinner` + etykieta (listy: `Skeleton` w fazie 5) |
| `Segmented` | 4 | `div role=group` + `aria-pressed` | `ButtonGroup` z `Button isSelected` (ADS nie ma segmented control w lustrze docs) |
| `Tabs` | 3 | `div role=tablist` bez paneli | `@atlaskit/tabs` (`Tabs` kontrolowane `selected`/`onChange`, `TabList`, `Tab`, `TabPanel`) — API prymitywu rozszerzone o `children` panelu |
| `Field` | 9 (41 wystąpień) | render-prop `{id, aria-describedby, aria-invalid}` | to samo API; `Label`, `HelperMessage`, `ErrorMessage` z `@atlaskit/form` |
| `ConfirmInline` | 6 | przycisk → „Tak / Anuluj” w miejscu | to samo API; `Button appearance="danger"` + `Button appearance="subtle"` |
| `CopyButton` | 2 | przycisk + `role=status` | `Button` z ikoną lucide `Copy` + `Tooltip`; komunikat jak dziś |
| *(nowe)* `Select`, `TextInput`, `TextArea`, `Checkbox`, `StatusLozenge`, `PageHeader` | — | natywne elementy w ekranach | cienkie opakowania ADS z API zbliżonym do natywnego (D5) |

Dodatkowo `ui/ui.css`: `.data-table`, `.table-wrap`, `.nowrap`, `.field*`, `.segmented`, `.tabs`, `.copy-status` — po fazie 2 zostają tylko `.data-table`/`.table-wrap`/`.nowrap` przepisane na tokeny.

## Goals / Non-Goals

**Goals:**
- Wygląd i zachowanie komponentów zgodne z ADS; zero surowych kolorów w `ui/src`; statusy wyłącznie przez `status.ts`.
- Ekrany zmieniają się minimalnie — migracja przede wszystkim przez prymitywy z zachowanym API.
- Gwarancje wymuszane przez `npm run lint` i `npm test`, nie tylko przez prompty agentów.
- Równoległa migracja ekranów w 4 grupach bez konfliktów plików.

**Non-Goals:**
- Zmiany backendu, kontraktu OpenAPI i `ui/src/api/*` (poza czystymi funkcjami pomocniczymi wydzielonymi do testów).
- Ręczny przełącznik motywu (motyw idzie za systemem), branding/logo, i18n.
- Cofanie zatwierdzenia po stronie serwera i zapis powodu odrzucenia (D11, D12).
- Testy komponentów w DOM (`jsdom`, `@testing-library/react`) — testujemy czystą logikę (D15).
- `@atlaskit/navigation-system` / `@atlassian/navigation-system` (D1).

## Decisions

Format: decyzja → uzasadnienie → odrzucone alternatywy.

### D1. Pakiety `@atlaskit/*` na React 18.3; powłoka na `page-layout` + `side-navigation`

`npm view <pkg> version peerDependencies` (2026-11-21):

| Pakiet | Wersja | peer `react` | Uwagi |
|---|---|---|---|
| `@atlaskit/tokens` | 20.14.0 | `^18.2.0 \|\| ^19.0.0` | `setGlobalTheme` (typy pakietu) |
| `@atlaskit/css-reset` | 8.4.10 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/primitives` | 22.8.0 | `^18.2.0 \|\| ^19.0.0` | zależy od `@emotion/react` i `@compiled/react` |
| `@atlaskit/css` | 1.1.10 | `^18.2.0 \|\| ^19.0.0` | nieużywany (D3) |
| `@atlaskit/button` | 25.8.3 | `^18.2.0 \|\| ^19.0.0` | import `@atlaskit/button/new` |
| `@atlaskit/lozenge` | 17.0.9 | `^18.2.0 \|\| ^19.0.0` | appearance: `default \| inprogress \| moved \| new \| removed \| success` |
| `@atlaskit/badge` | 22.0.0 | `^18.2.0 \|\| ^19.0.0` | liczniki |
| `@atlaskit/dynamic-table` | 19.3.9 | `^18.2.0 \|\| ^19.0.0` (+ react-dom) | |
| `@atlaskit/form` | 18.3.4 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/textfield` / `textarea` | 11.0.0 / 11.0.0 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/select` | 23.2.3 | `^18.2.0 \|\| ^19.0.0` (+ react-dom) | ciągnie `react-select` |
| `@atlaskit/checkbox` / `radio` / `toggle` | 19.3.3 / 10.2.7 / 17.3.6 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/tabs` | 21.3.1 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/flag` | 19.0.6 | `^18.2.0 \|\| ^19.0.0` | `FlagsProvider`, `useFlags` |
| `@atlaskit/modal-dialog` | 17.3.0 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/section-message` / `inline-message` | 10.3.0 / 16.4.9 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/spinner` / `skeleton` / `empty-state` | 20.4.4 / 4.4.4 / 12.2.4 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/heading` / `code` / `tooltip` / `tag` | 8.0.0 / 20.0.0 / 24.4.8 / 17.1.4 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/page-header` | 13.3.4 | `^18.2.0 \|\| ^19.0.0` | |
| `@atlaskit/page-layout` | 5.5.9 | `^18.2.0 \|\| ^19.0.0` (+ react-dom) | `PageLayout`, `LeftSidebar`, `Main`, `Content` |
| `@atlaskit/side-navigation` | 13.3.10 | `^18.2.0 \|\| ^19.0.0` | `SideNavigation`, `NavigationHeader`, `NavigationContent`, `Section`, `CustomItem` (prop `component` → `NavLink`), `NavigationFooter` |
| **`@atlaskit/navigation-system`** | 12.0.1 | **`^18.2.0`** (+ react-dom `^18.2.0`) | **niezgodny z React 19; mimo zejścia na 18.3 nie używamy (decyzja człowieka: page-layout + side-navigation)** |
| `@atlassian/navigation-system` | — | — | 404 w publicznym npm (lustro docs opisuje tę wersję) |
| `@atlaskit/eslint-plugin-design-system` | 16.16.1 | `^18.2.0 \|\| ^19.0.0` | preset `recommended/flat` istnieje |
| `lucide-react` | 1.77.0 | `^16.5.1 … \|\| ^19.0.0` | |

**Korekta po zadaniu 2.1 (decyzja człowieka): React 18.3.1 zamiast 19.** Pakiety bezpośrednie deklarują `^19.0.0`, ale ich zależności przechodnie w najnowszych wersjach (`latest`, brak nowszych tagów) mają peer tylko `^18.2.0`: `@atlaskit/analytics-next` 12.6.0 (`react`, `react-dom`), `@atlaskit/portal` 7.1.0 (`react`, `react-dom`), `@atlaskit/interaction-context` 4.1.0, `@atlaskit/browser-apis` 1.2.0. Na React 19 npm po cichu zagnieżdżał ~17 kopii `react-dom@18.3.1` (`npm ls` → `invalid`), a `@atlaskit/portal` (Flag, Modal, Tooltip) importuje z nich `createPortal` — załadowanie `react-dom@18` przy `react@19` kończy się błędem `Cannot read properties of undefined (reading 'ReactCurrentDispatcher')`. Kod UI nie używa API specyficznych dla React 19, `react-router-dom` 7 i `react-markdown` 10 mają peer `react >=18`. Wersje: `react`/`react-dom` 18.3.1, `@types/react` 18.3.31, `@types/react-dom` 18.3.7 (dokładne). Odrzucone: `overrides` (pakiety nie deklarują wsparcia React 19), `resolve.dedupe` w Vite (zostawia `invalid` w `npm ls`, maskuje problem tylko w bundlu).
Pozostałe ostrzeżenia `npm warn ERESOLVE overriding peer dependency` (bez flag, `npm ci` przechodzi): `react-scrolllock` 5.0.1 (zależność `@atlaskit/modal-dialog`, peer `react ^16.3.0`) i `@atlaskit/analytics-next-stable-react-context` 1.0.1 (peer `react ^16.8.0`) — przestarzałe zakresy, których nie spełnia żadna wersja React ≥ 17; `npm ls` pokazuje jedną fizyczną kopię `react`/`react-dom` 18.3.1, ale oznacza `react` jako `invalid` z powodu `react-scrolllock`.

Wniosek: poza `navigation-system` wszystkie pakiety bezpośrednie deklarują React 18 i 19; o wersji decydują zależności przechodnie (wyżej). Powłoka: `@atlaskit/page-layout` + `@atlaskit/side-navigation` (oba w lustrze docs: `pageLayout.md`) zamiast nowego systemu nawigacji.
Odrzucone: `@atlaskit/navigation-system` z `--legacy-peer-deps` (ryzyko błędów w runtime, `npm ci` w kontenerze bez flag); `@atlaskit/atlassian-navigation` (górny pasek produktów Atlassian — nadmiarowy dla 8 pozycji menu).

### D2. Ikony: `lucide-react`; `@atlaskit/icon` zakazany w naszym kodzie

Licencja grafiki ikon ADS (ADG) — decyzja człowieka: ikony z `lucide-react` (ISC). `@atlaskit/icon` i tak trafi do `node_modules`/bundla przechodnio (zależność `button`, `select`, `dynamic-table`, `flag`, `side-navigation`…) — tego nie da się uniknąć i nie łamie decyzji, bo nie renderujemy ikon ADS sami. Gwarancja: reguła `no-restricted-imports` dla `@atlaskit/icon`, `@atlaskit/icon/*`, `@atlaskit/icon-lab*` w `eslint.config.js`. `Flag` bez propsa `icon` pokazuje domyślną ikonę wg `appearance` (dopuszczalne — to część komponentu).
Ikony w przyciskach (`iconBefore`, `IconButton icon`) oczekują komponentu ikony ADS (props `label`, `color`). Adapter `ui/src/ui/icons.tsx`: funkcja opakowująca ikonę lucide (`aria-hidden`, `size={16}`, `color="currentColor"`) w komponent zgodny z tym typem. Zgodność typów weryfikuje `npm run typecheck` w zadaniu 2.6; gdy adapter nie wystarczy — ikona jako dziecko `Button` obok tekstu.
Wszystkie nazwy ikon w tabeli statusów sprawdzone w `lucide-react@1.77.0` (`dist/lucide-react.d.ts`); uwaga: w 1.x `AlertTriangle`/`TriangleAlert` to `TriangleExclamationPoint`, `ShieldAlert` → `ShieldExclamationPoint`, `AlertCircle` → `CircleExclamationPoint`.

### D3. Style: tokeny przez `token()` i `var(--ds-*)`; bez `@atlaskit/css`/`cssMap`

- Kolory, odstępy, promienie, typografia: w TS `token('color.text.subtle')` z `@atlaskit/tokens`, w plikach `.css` `var(--ds-…)`. Zero hex/rgb/hsl.
- Układ: `Box`, `Stack`, `Inline`, `Text` z `@atlaskit/primitives/compiled` z udokumentowanymi propsami tokenowymi (`space`, `padding`, `alignBlock`…, wg `box.md`, `stack.md`, `inline.md`) — bez `xcss`.
- Odrzucone: `@atlaskit/css` (`cssMap`) — wymaga transformacji Compiled w buildzie (`css.md`: „For configuration details, see our Get Started guide”), czyli dodatkowej konfiguracji Vite/Babel, której nie ma w lustrze docs; zysk niewielki przy 14 plikach CSS. `xcss` z `@atlaskit/primitives` — wg `css.md` zastępowany przez `@atlaskit/css`. Pozostała, niezastąpiona komponentami ADS, część CSS ekranów (siatki edytorów) zostaje w plikach `.css` na tokenach.
- Babel plugin `@atlaskit/tokens/babel-plugin` (fallbacki tokenów) — niepotrzebny: motyw ładujemy w runtime (D8).

### D4. Most zmiennych: stare zmienne CSS → tokeny ADS (faza 2)

`styles.css` przestaje definiować kolory hex; stare zmienne zostają jako aliasy tokenów, np. `--surface: var(--ds-surface)`, `--text: var(--ds-text)`, `--text-muted: var(--ds-text-subtle)`, `--border: var(--ds-border)`, `--accent: var(--ds-link)`, `--focus: var(--ds-border-focused)`, `--space-1: var(--ds-space-050)` …, a paleta reguł `--red`/`--red-bg` … `--gray`/`--gray-bg` → `var(--ds-text-accent-<kolor>)` / `var(--ds-background-accent-<kolor>-subtlest)`.
Skutek: po fazie 2 cała aplikacja (także niezmigrowane ekrany) jest w kolorach ADS i działa w ciemnym motywie; `paletteColor()` i `colors.test.ts` bez zmian (nadal zwracają `var(--red)`). W fazie 3 ekrany przestają używać zmiennych mostu (poza paletą reguł); w zadaniu 4.3 usuwa się nieużywane aliasy. Paleta reguł (`--red…--gray`) zostaje na stałe — to kolory nadawane w konfiguracji reguły (wyjątek od reguły czerwieni, spec: „Kolor z reguły Settlements zostaje”).
Odrzucone: migracja „wszystko naraz” bez mostu — niezmigrowane ekrany byłyby nieczytelne w ciemnym motywie do końca fazy 3.

### D5. Migracja przez prymitywy z zachowanym API

Mapa stary → ADS:

| Stary element | ADS | Pakiet / dokument | Uzasadnienie |
|---|---|---|---|
| powłoka `.layout` + `<nav>` (`App.tsx`) | `PageLayout` › `LeftSidebar` › `SideNavigation` (`NavigationHeader`/`Header` „Alerting”, `NavigationContent` z `Section` „Operacje” / „Konfiguracja”, `CustomItem component={NavLink}` z `isSelected`, `NavigationFooter` z `SessionBar`) + `Main` | `@atlaskit/page-layout`, `@atlaskit/side-navigation`; `pageLayout.md` | wzorzec Jira; skip-linki z `page-layout` |
| nagłówek ekranu (`<h1>` + przyciski) | `PageHeader` (`actions`, `bottomBar` dla filtrów) | `@atlaskit/page-header`; `pageHeader.md` | jednolite nagłówki |
| `<h1>`–`<h3>` | `Heading` (`size`, `as`) | `@atlaskit/heading`; `heading.md` | reguła lint `use-heading` |
| `button`, `.primary`, `.danger` | `Button` (`appearance="primary" \| "danger" \| "subtle"`, `isLoading`, `isDisabled`), `IconButton` (`label` wymagany) | `@atlaskit/button/new`; `button.md`, `iconButton.md` | `isLoading` zastępuje „Wysyłanie…” |
| link wewnętrzny `<Link>` (react-router) | bez zmian (`Link` z react-router, kolor z tokenów `color.link` w CSS) | — | integracja `@atlaskit/link` z routerem wymaga `AppProvider`, którego nie ma w lustrze docs; `<Link>` to komponent, nie `<a>`, więc `no-html-anchor` go nie zgłasza |
| link zewnętrzny `<a>` (Markdown runbooków) | `Link` z `@atlaskit/link` (`href`, `target`) | `@atlaskit/link` 5.1.7 (peer `^19.0.0`); `link.md` | reguła `no-html-anchor` |
| `Badge` / `*Badge` | `StatusLozenge` = `Lozenge` + ikona lucide + etykieta z `status.ts` | `@atlaskit/lozenge`; `lozenge.md` | D6 |
| liczniki (`×12`, „Wszystkie (5)”) | `Badge` | `@atlaskit/badge`; `badge.md` | |
| `Card` | `Box` (tokeny `elevation.surface.raised`, `border`) + `Heading` + `Spinner` | `@atlaskit/primitives/compiled`, `@atlaskit/heading`, `@atlaskit/spinner` | ADS nie ma komponentu Card |
| `ErrorBox`, `.error-box` | `SectionMessage appearance="error"` (`actions`) | `@atlaskit/section-message`; `sectionMessage.md` | |
| `.rules-saved`, `.rules-banner`, komunikaty „zapisano” | `Flag` przez `useFlags().showFlag` (sukces, auto-dismiss) / `SectionMessage appearance="warning"` | `@atlaskit/flag`; `flag.md`, `flagsProvider.md` | spec „Powiadomienie o wyniku akcji” |
| `Empty` | tekst stonowany (`Text color="color.text.subtlest"`); pusty ekran → `EmptyState` | `@atlaskit/empty-state`; `emptyState.md` | |
| `Loading` | `Spinner` (`label`), listy → `Skeleton` | `@atlaskit/spinner`, `@atlaskit/skeleton` | |
| `Segmented` | `ButtonGroup` + `Button isSelected` | `@atlaskit/button`; `button.md` (isSelected), `buttonGroup.md` | brak segmented control w ADS |
| `Tabs` | `Tabs` (`selected`, `onChange`, `id`) + `TabList`/`Tab`/`TabPanel` | `@atlaskit/tabs`; `tabs.md` | poprawne `tabpanel` |
| `Field` | `Label` + `HelperMessage` + `ErrorMessage` (zachowany render-prop) | `@atlaskit/form`; `form.md` | API prymitywu bez zmian; pełny `Form`/`Field` z ADS odrzucony — przepisanie logiki walidacji edytorów |
| `<input type=text>` | `TextInput` → `Textfield` | `@atlaskit/textfield`; `textField.md` | przyjmuje natywne propsy |
| `<textarea>` (YAML, payload) | `TextArea` → `TextArea` | `@atlaskit/textarea`; `textArea.md` | |
| `<select>` | `Select` (prymityw: `value: string`, `options`, `onChange(value)`) → `@atlaskit/select` | `select.md` | ekrany zmieniają tylko import i `onChange` |
| checkbox / radio | `Checkbox` / `Radio`, `RadioGroup` | `@atlaskit/checkbox`, `@atlaskit/radio` | |
| przełącznik kanału (MockOutbox) | `Toggle` (`isChecked`, `onChange`, `label`) | `@atlaskit/toggle`; `toggle.md` | stan nadal jako `StatusLozenge` |
| `ConfirmInline` | to samo API na `Button`; odrzucenie z klawiatury / zbiorcze → `ModalDialog` | `@atlaskit/modal-dialog`; `modalDialog.md` | |
| `<code>`, `.mono` | `Code` | `@atlaskit/code`; `code.md` | reguła `no-html-code` |
| `<pre>` (payload, YAML) | `CodeBlock` (tylko odczyt) | `@atlaskit/code`; `codeBlock.md` | |
| `title=` jako podpowiedź | `Tooltip` | `@atlaskit/tooltip`; `tooltip.md` | |
| feed alertów `<table>` | `DynamicTable` (`head`, `rows`, `isLoading`, `emptyView`, `onRowClick`, `highlightedRowIndex`) | `@atlaskit/dynamic-table`; `dynamicTable.md` | stany ładowania/pusty wbudowane; `highlightedRowIndex` dla `j`/`k` |
| małe tabele (Audit, Episodes, Deliveries, Outbox, Actions, Mappings, People) | natywna `<table class="data-table">` przepisana na tokeny | — | struktura `head/rows` DynamicTable to przepisanie 7 tabel bez zysku; brak reguły lint na `<table>` |
| `<details>` (runbooki, payload) | bez zmian (natywne, tokeny w CSS) | — | brak odpowiednika w lustrze docs poza `Expand`/nawigacją |

### D6. `ui/src/ui/status.ts` — jedno źródło statusów

Kształt: `STATUS: Record<Domain, Record<string, { label: string; appearance: ThemeAppearance; isBold?: boolean; icon: LucideIcon }>>` + `statusOf(domain, value)` z bezpiecznym fallbackiem (`default`, `CircleDashed`, etykieta = surowa wartość). Renderuje wyłącznie `StatusLozenge` (prymityw) i `Badges.tsx` (delegacja, API bez zmian). Reguła czerwieni (`appearance: "removed"`) dozwolona tylko dla wartości oznaczonych w tabeli jako R; test `status.test.ts` sprawdza listę dozwolonych wartości `removed`.

Tabela statusów (`R` = dozwolona czerwień: odrzucone / blokujące / porażka / przekroczony termin):

| Domena (`Schemas[...]`) | Wartość | Etykieta PL | Lozenge `appearance` | `isBold` | Ikona lucide | R | Uzasadnienie |
|---|---|---|---|---|---|---|---|
| `alertState` (`AlertState`) | `open` | otwarty | `moved` | — | `BellRing` | | wymaga uwagi (`lozenge.md`: moved = „require attention”); dziś czerwony |
| | `reopened` | ponownie otwarty | `moved` | tak | `RotateCcw` | | wraca problem — mocniej niż `open` |
| | `acked` | potwierdzony | `inprogress` | — | `Eye` | | ktoś się zajmuje |
| | `resolved` | rozwiązany | `success` | — | `CircleCheck` | | |
| `severity` (`Severity`) | `critical` | krytyczny | `removed` | tak | `Siren` | R (blokujące) | krytyczny = blokuje produkcję; spójne z dzisiejszym UI |
| | `error` | błąd | `moved` | tak | `TriangleExclamationPoint` | | |
| | `warning` | ostrzeżenie | `moved` | — | `CircleExclamationPoint` | | |
| | `info` | informacja | `default` | — | `Info` | | |
| `delivery` (`DeliveryStatus`) | `pending` | oczekuje | `default` | — | `Clock` | | |
| | `retrying` | ponawianie | `moved` | — | `RefreshCw` | | |
| | `delivered` | dostarczono | `success` | — | `CheckCheck` | | |
| | `fallback` | kanał zapasowy | `inprogress` | — | `ArrowRightLeft` | | łańcuch trwa; dziś pomarańczowy |
| | `failed` | błąd | `removed` | — | `CircleX` | R (porażka) | |
| | `dlq` | DLQ | `removed` | tak | `ArchiveX` | R (porażka) | |
| `step` (`StepStatus`) | `pending` | oczekuje | `default` | — | `Circle` | | |
| | `running` | w toku | `inprogress` | — | `LoaderCircle` | | bez animacji (prefers-reduced-motion) |
| | `succeeded` | zrobione | `success` | — | `CircleCheck` | | |
| | `failed` | błąd | `removed` | — | `CircleX` | R (porażka) | |
| | `waiting_approval` | czeka na zatwierdzenie | `moved` | tak | `Hand` | | blokuje przebieg do decyzji człowieka, ale to nie porażka |
| | `waiting_manual` | krok ręczny | `moved` | — | `UserRound` | | |
| | `skipped` | pominięty | `default` | — | `SkipForward` | | |
| | `rejected` | odrzucony | `removed` | — | `Ban` | R (odrzucone) | |
| `risk` (`ActionDef.risk`) | `high` | ryzyko: wysokie | `moved` | tak | `ShieldExclamationPoint` | | wymaga zatwierdzenia — uwaga, nie porażka; **zmiana: dziś czerwony** |
| | `low` | ryzyko: niskie | `default` | — | `ShieldCheck` | | dziś zielony — „niskie ryzyko” to nie sukces |
| `approval` (`StepDef.approval`) | `required` | wymaga zatwierdzenia | `moved` | — | `ShieldQuestionMark` | | |
| | `none` | — (nie renderujemy) | — | — | — | | |
| `approvalDeadline` | `overdue` | przekroczony termin | `removed` | tak | `TimerOff` | R (termin) | **zarezerwowane** — kontrakt nie ma terminu/SLA zatwierdzenia; nieużywane do decyzji człowieka (pytanie Q4) |
| `channelAvailability` (`MockChannelState.available`) | `true` | włączony | `success` | — | `Wifi` | | |
| | `false` | awaria | `removed` | tak | `WifiOff` | R (porażka) | celowo symulowana awaria |
| `channel` (`Channel`, kategoria) | `ui` / `email` / `sms` / `phone` | UI / e-mail / SMS / telefon | `default` | — | `Monitor` / `Mail` / `MessageSquare` / `Phone` | | kategoria, nie stan |
| `rule` (`Rule.enabled`) | `true` | włączona | `success` | — | `CirclePlay` | | |
| | `false` | wyłączona | `default` | — | `CirclePause` | | |
| `stream` (`StreamStatus`) | `idle` | rozłączono | `default` | — | `Unplug` | | |
| | `connecting` | łączenie… | `inprogress` | — | `LoaderCircle` | | |
| | `live` | na żywo | `success` | — | `Radio` | | |
| | `reconnecting` | ponowne łączenie… | `moved` | — | `RefreshCw` | | |
| | `unavailable` | na żywo niedostępne (mock) | `default` | — | `WifiOff` | | mock bez SSE to nie awaria |
| `runbookKind` | `automatic` / `manual` | automatyczny / ręczny | `new` / `default` | — | `Bot` / `BookOpen` | | kategoria |
| `stepAutomation` | `automated` / `manual` | automatyczny / do wykonania ręcznie | `new` / `moved` | — | `Bot` / `Hand` | | `manual` przy akcji = brak w rejestrze → uwaga |
| `actionKind` | `builtin` / `http` | wbudowana / HTTP | `default` / `new` | — | `Lock` / `Zap` | | kategoria |
| `schema` | `unmapped` | UNMAPPED | `moved` | — | `CircleQuestionMark` | | wymaga mappingu |
| `action` | `missing` | brak w rejestrze | `moved` | — | `CircleQuestionMark` | | |
| *(fallback)* | dowolna nieznana | surowa wartość | `default` | — | `CircleDashed` | | spec: „Nieznany status z backendu” |

Elementy, które dziś sygnalizują stan samym kolorem i przechodzą na `StatusLozenge` (lub znikają): `.ad-step-*` (krawędź kroku), `.approval-high`, `.channel-ok/.channel-down`, `.stream-*`, `.ad-error-text` (zostaje tekst błędu + `StatusLozenge` statusu obok), `.condition-op.not` (bez czerwieni), `.rules-saved` (→ Flag).

### D7. Gwarancje w kodzie: ESLint + test na surowe kolory

- `ui/eslint.config.js` (flat config, ESLint 9): `typescript-eslint` (parser TSX), `@atlaskit/eslint-plugin-design-system` z `configs['recommended/flat']` (potwierdzone w `dist/cjs/index.codegen.js` pakietu 16.16.1; zawiera m.in. `ensure-design-token-usage`, `no-html-button`, `no-html-select`, `no-html-text-input`, `no-html-heading`, `no-html-code`, `no-html-anchor`, `use-heading`, `icon-label`, `no-unsafe-design-token-usage`), plus `no-restricted-imports` dla `@atlaskit/icon*`, `@atlaskit/navigation-system`, `@atlassian/*`, `@atlaskit/css`. Pomijamy `src/api/schema.d.ts`.
- `package.json`: `"lint": "eslint src"`. W fazie 2 błędy (`error`) blokują, ostrzeżenia `no-html-*` w niezmigrowanych ekranach są dozwolone; każda grupa fazy 3 doprowadza swoje pliki do `npx eslint <pliki> --max-warnings 0`; zadanie 3.5 zmienia skrypt na `eslint src --max-warnings 0`.
- `ensure-design-token-usage` nie analizuje plików `.css`, więc dochodzi test `ui/src/ui/no-raw-colors.test.ts` (vitest, środowisko `node`, `fs`): żaden plik `ui/src/**/*.{css,ts,tsx}` poza `api/schema.d.ts` i samym testem nie zawiera `#[0-9a-fA-F]{3,8}\b`, `rgb(`, `rgba(`, `hsl(`. Działa od fazy 2 (po D4 hex zostaje tylko w historii git).
- Odrzucone: stylelint (kolejna zależność dla jednej reguły); sam prompt agenta.

### D8. Motyw: `setGlobalTheme({ colorMode: 'auto' })` + `@atlaskit/css-reset`

`main.tsx`: `import '@atlaskit/css-reset'`; `setGlobalTheme({ colorMode: 'auto', light: 'light', dark: 'dark', spacing: 'spacing', typography: … })` (sygnatura z `set-global-theme.d.ts`: ustawia `data-theme`/`data-color-mode` na `<html>`, `auto` = ustawienie systemu, zwraca `Promise<UnbindFn>`). Render dopiero w `.then/.finally` obietnicy — bez top-level `await` (domyślny target buildu Vite nie musi wspierać TLA) — żeby uniknąć mignięcia bez tokenów. Wartość `typography` wybrać z `ThemeIdsWithOverrides` w typach pakietu (zadanie 2.2).
Odrzucone: ręczny przełącznik motywu (poza zakresem; łatwy do dodania później przez to samo API).

### D9. Klawiatura i paleta komend (faza 5, wzorce power-ui, wygląd ADS)

- `ui/src/ui/keyboard.ts`: czysta funkcja `isTypingTarget(el)` (input, textarea, select, contenteditable, `[role=combobox]`, wnętrze `[role=dialog]`) + hook `useListKeyboard({ count, enabled, onOpen, onAction })` z indeksem w `useRef` (uwaga z power-ui o nieaktualnym domknięciu) i warstwami przez `enabled` (Modal > paleta > lista).
- Skróty: `j`/`k`/strzałki, `Enter`, `x`, `a`, `r`, `Escape`, `⌘K`/`Ctrl+K`. Bez warstwy `g`+litera (nie zamówiona).
- Paleta: `ModalDialog` + `Textfield` + `@atlaskit/menu` `ButtonItem`; źródła: pozycje menu + alerty z `useAlertFeed`. Bez nowej zależności typu `cmdk`.
- Wiersz bieżący: `highlightedRowIndex` w `DynamicTable` (feed) / token `color.background.selected` (kolejka) + `focus-visible`.

### D10. Akcje zbiorcze bez zmiany kontraktu

Brak endpointu zbiorczego → pętla po istniejącym `POST /runs/{run_id}/steps/{step_id}/{command}`, sekwencyjnie (prostsze komunikaty i brak burstu do Kafki), wynik per krok w istniejącej mapie `rowErrors`. Logika wyboru i agregacji wyników jako czysta funkcja w `ui/src/screens/approvals/bulk.ts` (testowalna w `node`). Pasek akcji zbiorczych: `Inline` + `Button`y + `Badge` z liczbą zaznaczonych.

### D11. „Cofnij” po zatwierdzeniu — nie w tym change'u (domyślnie)

Kontrakt nie ma komendy cofnięcia (`command` ∈ `approve | reject | done | retry | skip | manual_mode`), a runbook po `approve` od razu wykonuje akcję. Zgodnie z definicją agenta `approval-ux`: Flag z wynikiem i odnośnikiem do alertu, bez „Cofnij”; odrzucenie z potwierdzeniem.
Alternatywa (do decyzji człowieka, Q2): „opóźnione wysłanie” — komenda wysyłana po 5 s, Flag z „Cofnij” anuluje ją przed wysłaniem. Wady: decyzja ginie przy zamknięciu karty w oknie 5 s, inni operatorzy widzą krok jako oczekujący dłużej.

### D12. Powód odrzucenia — nie zbieramy (domyślnie)

`requestBody.note` jest opisany „dla `done`”, a `crates/api/src/routes/runbooks.rs` (l. 179–188) przekazuje notatkę tylko do `RunbookCommandKind::Done`. Pole „powód” przy odrzuceniu byłoby tracone — wprowadzałoby operatora w błąd. Domyślnie `r` i odrzucenie zbiorcze otwierają `ModalDialog` z przyciskiem `appearance="danger"` bez pola powodu. Zapis powodu wymaga osobnego change'a (kontrakt + `runbooks` + dziennik) — pytanie Q3.

### D13. Testy w środowisku `node`, bez DOM

`vitest.config.ts` zostaje (`environment: "node"`, `src/**/*.test.ts`). Testowalna logika jest wydzielana do czystych modułów: `status.ts`, `keyboard.ts` (`isTypingTarget` na prostych obiektach-atrapach), `approvals/bulk.ts`, `no-raw-colors.test.ts`. Istniejące 139 przypadków `it(...)` (147 testów wg ostatniego raportu vitest) przechodzi bez zmiany asercji. Dodanie `jsdom` + `@testing-library/react` — tylko za zgodą człowieka (Q5).

### D14. Rozmiar bundla

Zadanie 2.1 zapisuje wynik `npm run build` przed zmianami (rozmiary chunków z wyjścia Vite) w `openspec/changes/ui-atlassian-design/bundle.md`; każde kolejne zadanie z buildem dopisuje różnicę. Importy tylko z punktów wejścia pakietów (`@atlaskit/button/new`, nie całe barrel-e), ikony lucide nazwane (tree-shaking). Jeśli główny chunk przekroczy próg ostrzeżenia Vite (500 kB po minifikacji), ekrany konfiguracji (`Rules`, `Mappings`, `Actions`) ładowane przez `React.lazy` + `Suspense` ze `Spinner` (zadanie 3.5).

### D15. Podział pracy: fazy i agenci

| Faza | Agent(y) | Pliki | Równoległość |
|---|---|---|---|
| 1 plan | `ui-auditor` | `openspec/changes/ui-atlassian-design/` | — |
| 2 fundament | `ads-foundation` (1 agent) | `ui/package*.json`, `ui/eslint.config.js`, `ui/src/main.tsx`, `ui/src/App.tsx`, `ui/src/styles.css`, `ui/src/ui/*`, `ui/src/components/*` | sekwencyjnie |
| 3 ekrany | `screen-migrator` ×4 (osobne worktree) + `test-writer` | wyłącznie pliki swojej grupy (tabela niżej) | 4 grupy + testy równolegle |
| 4 review | `design-guardian` + `code-reviewer` (read-only) | — | równolegle |
| 5 UX zatwierdzeń | `approval-ux` | `Approvals.tsx`, `AlertFeed.tsx`, `ui/src/ui/keyboard.ts`, `screens/approvals/*`, paleta w `ui/src/ui/` + montaż w `App.tsx` | po fazie 4 |

**Uwaga:** definicja `ads-foundation` nie wymienia `ui/src/components/*`, a `screen-migrator` ma je zakazane. Komponenty współdzielone (`Badges`, `Presentation`, `Markdown`, `ConditionView`, `PayloadTree`, `SourceFile`, `SessionBar`, `StreamIndicator`, `colors`) muszą więc trafić do fazy 2 — wymaga to rozszerzenia zakresu `ads-foundation` o `ui/src/components/*` (pytanie Q1). Bez tego żadna grupa fazy 3 nie może ich ruszyć.

Grupy fazy 3 (żaden plik nie należy do dwóch grup; importy między grupami — brak, sprawdzone `grep`):

| Grupa | Pliki | Linie TSX | Uwagi |
|---|---|---|---|
| **G1 Alerty** | `AlertFeed.tsx/.css`, `AlertDetail.tsx/.css`, `alert-detail/{Audit,Deliveries,Episodes,Runbooks}.tsx`, `DevComponents.tsx` | ~812 | `DynamicTable` w feedzie; łańcuch fallbacku w `Deliveries.tsx` 1:1 |
| **G2 Operacje + Akcje** | `Approvals.tsx/.css`, `MockOutbox.tsx/.css`, `Actions.tsx/.css`, `actions/ActionForm.tsx` | ~921 | `Approvals` tylko wizualnie (UX w fazie 5); `Toggle` kanału |
| **G3 Reguły** | `Rules.tsx/.css`, `rules/{RuleEditor,ConditionBuilder,RuleTester}.tsx` | ~1164 | najwięcej formularzy; `rules/model.ts` + testy bez zmian |
| **G4 Mappingi + konfiguracja tylko do odczytu** | `Mappings.tsx/.css`, `mappings/{MappingEditor,PreviewPanel}.tsx`, `Runbooks.tsx/.css`, `People.tsx/.css` | ~1345 | `mappings/{draft,localPreview,sample}.ts` + testy bez zmian |

`test-writer` równolegle do fazy 3: tylko nowe `*.test.ts` w `ui/src/ui/` (pliki nienależące do żadnej grupy).

## Risks / Trade-offs

- [React i peer deps] **Zmaterializowane w 2.1:** przechodnie `@atlaskit/{analytics-next,portal,interaction-context,browser-apis}` mają peer tylko `^18.2.0`, a zagnieżdżony `react-dom@18` wywracał się z React 19. → Decyzja człowieka: zejście na React 18.3.1 (D1). Weryfikacja 2.1: `npm install` bez `--legacy-peer-deps` i bez `overrides`, jedna fizyczna kopia `react`/`react-dom` 18.3.1, `rm -rf node_modules && npm ci` przechodzi; zostają ostrzeżenia peer `react-scrolllock` (`^16.3.0`) i `analytics-next-stable-react-context` (`^16.8.0`) nie do spełnienia przez żadną wersję React ≥ 17. Powrót na React 19 — gdy Atlassian wyda te pakiety z peer `^19`.
- [Runtime `@compiled/react` bez pluginu buildu] Pakiety ADS są dystrybuowane jako skompilowany JS ze stylami w runtime; nasz kod nie używa `css`/`cssMap`, więc plugin Compiled nie jest potrzebny. → Weryfikacja w 2.4: `/dev/components` renderuje `Button`, `Lozenge`, `Box` ze stylami w `npm run build && npm run preview`.
- [ESLint 9 vs plugin] Plugin ma w devDependencies `eslint ^8.57.0`, ale eksportuje preset flat. → Zadanie 2.5 sprawdza, że `npm run lint` działa na ESLint 9; jeśli nie — ESLint 8.57 + `.eslintrc.cjs` z `plugin:@atlaskit/design-system/recommended` (README pakietu).
- [Typy ikon ADS vs lucide] `IconButton`/`iconBefore` oczekują typu ikony ADS. → Adapter `ui/icons.tsx` (D2), weryfikacja `npm run typecheck` w 2.6.
- [Rozmiar bundla] ADS jest ciężki (emotion + compiled + react-select). → Pomiar przed/po (D14), lazy-loading ekranów konfiguracji powyżej progu.
- [Zmiana kolorów znanych z demo] `open` przestaje być czerwony, `risk: high` przestaje być czerwony. → To wymóg reguły czerwieni; `docs/DEMO.md` nie odwołuje się do kolorów statusów (sprawdzić w 4.4).
- [Konflikty przy scalaniu worktree] Grupy nie dzielą plików, ale wszystkie zależą od API prymitywów z fazy 2. → Faza 3 startuje dopiero po zamknięciu fazy 2 i scaleniu jej do `feat/ui-ads-refactor`; zmiany wspólnych plików, których grupa potrzebuje, trafiają do raportu, nie do kodu (definicja `screen-migrator`), i są robione w zadaniu 3.5.
- [Dostępność `DynamicTable` z klikalnym wierszem] Klik w wiersz nie jest dostępny z klawiatury. → Tytuł alertu zostaje linkiem; `Enter` z `j`/`k` w fazie 5.
- [Ciemny motyw a kolory z reguł] Paleta reguł mapowana na tokeny akcentowe ADS zmienia się z motywem — zamierzone.

## Migration Plan

1. Faza 2 na gałęzi `feat/ui-ads-refactor` — po niej aplikacja w pełni działa (most D4), lint i test kolorów zielone.
2. Faza 3: 4 worktree odgałęzione od wyniku fazy 2, każdy z commitem; scalanie do `feat/ui-ads-refactor` kolejno G1 → G2 → G3 → G4 (konflikty niemożliwe przy rozłącznych plikach), po każdym scaleniu `npm run typecheck && npm run lint && npm test && npm run build`. Potem zadanie 3.5 (lint `--max-warnings 0`, lazy-loading).
3. Faza 4 — raporty, poprawki blokujących znalezisk wracają jako zadania do właściwej grupy/agenta.
4. Faza 5, potem PR do `main`.
Wycofanie: revert scalenia gałęzi; zmian w danych/kontrakcie brak, więc rollback jest czysto front-endowy.

## Open Questions

- **Q1 (blokuje start fazy 2 w obecnym brzmieniu agentów):** zgoda na rozszerzenie zakresu `ads-foundation` o `ui/src/components/*` (D15). Alternatywa: osobne zadanie dla człowieka/agenta ogólnego.
- **Q2:** „Cofnij” po zatwierdzeniu przez opóźnione wysłanie komendy (D11) — domyślnie NIE.
- **Q3:** zapis powodu odrzucenia (rozszerzenie kontraktu i `runbooks`, osobny change) — domyślnie NIE, odrzucenie z potwierdzeniem bez powodu (D12).
- **Q4:** czy wprowadzić próg „przekroczony termin” dla zatwierdzeń liczony w UI z `waiting_since` (np. > 15 min) — domyślnie NIE (brak SLA w kontrakcie; status zarezerwowany w `status.ts`).
- **Q5:** zgoda na `jsdom` + `@testing-library/react` dla testów komponentów — domyślnie NIE (D13).
- **Q6:** severity `critical` jako czerwony (`removed`, „blokujące”) — przyjęte; potwierdzić, że to zgodne z intencją reguły czerwieni.
