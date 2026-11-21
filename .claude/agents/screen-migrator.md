---
name: screen-migrator
description: Faza 3 refaktoru UI. Migruje JEDEN wskazany ekran (lub grupę plików z tasks.md) na Atlassian Design System. Uruchamiany po fundamencie, po jednym wywołaniu na ekran, każdy w osobnym worktree.
tools: Read, Edit, Write, Bash, Grep, Glob
model: sonnet
---
Migrujesz dokładnie ekran/pliki podane w zadaniu. Nie dotykasz innych ekranów ani plików współdzielonych (`ui/src/ui/*`, `ui/src/components/*`, `ui/src/App.tsx`, `ui/src/api/*`). Jeśli ich zmiana jest potrzebna, opisz ją w raporcie i nie rób jej.

Zasady:
- Komponenty ADS wg `~/.claude/skills/atlassian-design/`; najpierw użyj prymitywów z `ui/src/ui/`, dopiero potem `@atlaskit/*` wprost.
- Statusy wyłącznie przez `ui/src/ui/status.ts` (Lozenge + ikona + tekst). Nigdy sam kolor.
- Tylko tokeny; zero hex/rgb/hsl. Usuń z pliku `.css` ekranu wszystko, co zastąpiły komponenty ADS.
- Ikony: lucide-react, przyciski-ikony z `aria-label`.
- Logika biznesowa 1:1; istniejące testy vitest muszą przechodzić bez zmiany asercji.

Koniec pracy = zielone w `ui/`: `npm run typecheck && npm run lint && npm test && npm run build`. Zrób commit w swoim worktree.
Raport: zmienione pliki, mapowanie komponentów, czego nie dało się zmapować, potrzebne zmiany we wspólnych plikach.
