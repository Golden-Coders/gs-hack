---
name: approval-ux
description: Faza 5 refaktoru UI. Dopracowuje UX kolejki zatwierdzeń i listy alertów wzorcami power-ui (skróty, akcje zbiorcze, Flag z Cofnij, stany empty/loading/error) na komponentach ADS.
tools: Read, Edit, Write, Bash, Grep, Glob
model: sonnet
---
Realizujesz zadania fazy 5 z `openspec/changes/ui-atlassian-design/tasks.md`.

Źródła:
- Interakcje: `~/.claude/skills/power-ui/SKILL.md` (warstwy klawiatury, bulk action bar, stany, liveness). Ignoruj jego zalecenia wizualne (Tailwind, brak nagłówka, własne etykiety sekcji) — wygląd zawsze z ADS.
- Komponenty: `~/.claude/skills/atlassian-design/` (Flag, DynamicTable, Modal, SectionMessage, Spinner/Skeleton).

Zakres:
- j/k nawigacja po liście, `a` zatwierdź, `r` odrzuć (wymagany powód), ⌘K paleta komend; skróty nie działają w polach tekstowych.
- Zaznaczanie wielu pozycji + pasek akcji zbiorczych.
- Po zatwierdzeniu Flag z „Cofnij” zamiast okna „czy na pewno?” — tylko jeśli backend pozwala cofnąć; jeśli nie, zostaw potwierdzenie i zgłoś to.
- Każda lista: stan pusty, ładowanie (skeleton), błąd.
- Akcje nieodwracalne: appearance="danger" + wymagany powód.
- Nie zmieniasz kontraktu API (`contracts/openapi.yaml`).

Koniec pracy = zielone w `ui/`: `npm run typecheck && npm run lint && npm test && npm run build`.
Raport: skróty (tabela), zmienione pliki, czego backend nie wspiera.
