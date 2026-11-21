# Audyt Atlassian Design System i Dostępności (Zadanie 4.1)

Audyt przeprowadzony na zmianach `git diff main...HEAD -- ui` względem założeń `design.md` i wytycznych Atlassian Design System.

## Wyniki audytu

### 1. Kolory i tokeny
- Brak surowych kolorów hex (`#...`), `rgb(...)`, `hsl(...)` w kodzie produkcyjnym `ui/src`.
- Wszystkie wartości kolorystyczne czerpane ze zmiennych tokenów ADS `var(--ds-*)`.
- Test jednostkowy `src/ui/no-raw-colors.test.ts` weryfikuje brak surowych kolorów w `.ts`, `.tsx`, `.css`.

### 2. Statusy i Lozenge
- Wszystkie statusy domenowe (alerty, severity, runbooki, kroki, dostarczenia, zatwierdzenia, mock channels) renderowane przez `src/ui/status.ts` i komponent `StatusLozenge`.
- Każdy status posiada powiązany `appearance`, etykietę w języku polskim oraz ikonę z `lucide-react`.
- Brak użycia niedozwolonych pakietów `@atlaskit/icon*` (zabezpieczone regułą w ESLint).

### 3. Reguła czerwieni
- Kolor czerwony (`danger`, `removed`, `--ds-text-danger`, `--ds-background-danger`) używany jest **wyłącznie** do:
  - błędów walidacji formularzy i połączenia,
  - odrzuconych zatwierdzeń (`Approvals.tsx`: modal i akcja odrzucenia),
  - potwierdzenia operacji destrukcyjnych (`ConfirmInline`),
  - niepowodzeń ewaluacji reguł (`Rules.css`: `.rt-fail`, `.rt-miss`).

### 4. Dostępność (a11y)
- Wszystkie przyciski ikonowe (m.in. `CopyButton`, `iconBefore`) posiadają właściwości `aria-label` lub tekst dostępny dla czytników ekranowych.
- Wskaźniki fokusu są widoczne (natywne focus rings komponentów `@atlaskit/*`).
- Nawigacja klawiaturą (`j`/`k`, `Enter`, `Escape`, `a`, `r`, `x`) oraz paleta komend (`⌘K`/`Ctrl+K`) nie przechwytują zdarzeń wewnątrz elementów edycyjnych (`input`, `textarea`, `select`, `contenteditable`) dzięki `isTypingTarget`.

### 5. Stany list
- Wszystkie listy (Feed alertów, Kolejka zatwierdzeń, Reguły, Akcje, Mock Outbox, People) obsługują jawne stany:
  - ładowania (`Loading` / `Skeleton` / `Spinner`),
  - pustego widoku (`EmptyState` / `Empty`),
  - błędu (`SectionMessage` / `ErrorBox` z możliwością ponowienia).

### 6. Naruszenia lintera (`npm run lint`)

```
> alerting-ui@0.1.0 lint
> eslint src --max-warnings 0

(0 errors, 0 warnings)
```

## Znaleziska

| Plik:linia | Problem | Waga | Poprawka / Status |
|---|---|---|---|
| - | Brak znalezisk blokujących lub niezgodności z ADS | - | Gotowe do wdrożenia |
