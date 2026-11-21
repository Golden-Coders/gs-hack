// Skróty klawiszowe list (design.md D9). `isTypingTarget` jest czyste (bez DOM) — testowalne atrapami w node.
import { useEffect, useRef, useState } from "react";

/** Minimalny kształt elementu DOM potrzebny do decyzji; prawdziwy `Element` go spełnia. */
export type KeyTarget = {
  tagName?: string;
  isContentEditable?: boolean;
  closest?: (selector: string) => unknown;
};

// Pola tekstowe, listy wyboru, kombo (react-select) i wnętrze okien dialogowych (modal > paleta > lista).
const TYPING_SELECTOR = '[role="combobox"], [role="dialog"], [contenteditable=""], [contenteditable="true"]';

/** Czy zdarzenie klawiatury pochodzi z miejsca, w którym jednoklawiszowe skróty muszą ustąpić. */
export function isTypingTarget(el: KeyTarget | null | undefined): boolean {
  if (!el) return false;
  const tag = (el.tagName ?? "").toUpperCase();
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (el.isContentEditable) return true;
  return Boolean(el.closest?.(TYPING_SELECTOR));
}

export type ListKeyboardOptions = {
  /** Stabilne klucze wierszy w kolejności wyświetlania — bieżący wiersz jest śledzony po kluczu, nie po pozycji. */
  keys: string[];
  /** `false` = warstwa nadrzędna (modal, paleta) przejmuje klawisze. */
  enabled: boolean;
  onOpen?: (key: string) => void;
  /** Klawisze `x`, `a`, `r` i `Escape`; `key` bieżącego wiersza lub `null`. */
  onAction?: (action: "x" | "a" | "r" | "Escape", key: string | null) => void;
};

/**
 * j/k i strzałki przesuwają bieżący wiersz, Enter otwiera, x/a/r/Escape trafiają do `onAction`.
 * Stan bieżącego wiersza trzyma `useRef` (listener nie widzi starych domknięć); `useState` tylko odświeża widok.
 */
export function useListKeyboard({ keys, enabled, onOpen, onAction }: ListKeyboardOptions) {
  const keysRef = useRef(keys);
  const activeRef = useRef<string | null>(null);
  const lastIndexRef = useRef(0);
  const handlers = useRef({ onOpen, onAction });
  const [, setTick] = useState(0);
  keysRef.current = keys;
  handlers.current = { onOpen, onAction };

  // Bieżący wiersz zniknął (np. zatwierdzony) → najbliższy sąsiad na tej samej pozycji.
  let index = activeRef.current === null ? -1 : keys.indexOf(activeRef.current);
  if (activeRef.current !== null && index < 0) {
    index = Math.min(lastIndexRef.current, keys.length - 1);
    activeRef.current = keys[index] ?? null;
  }
  if (index >= 0) lastIndexRef.current = index;
  const activeKey = index >= 0 ? (keys[index] ?? null) : null;

  useEffect(() => {
    if (!enabled) return;
    const set = (key: string | null) => {
      activeRef.current = key;
      setTick((n) => n + 1);
    };
    const move = (delta: number) => {
      const ks = keysRef.current;
      if (ks.length === 0) return;
      const cur = activeRef.current === null ? -1 : ks.indexOf(activeRef.current);
      const next = cur < 0 ? (delta > 0 ? 0 : ks.length - 1) : Math.min(ks.length - 1, Math.max(0, cur + delta));
      set(ks[next] ?? null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target as Element | null;
      if (isTypingTarget(target)) return;
      const key = e.key;
      const { onOpen: open, onAction: act } = handlers.current;
      if (key === "j" || key === "ArrowDown") move(1);
      else if (key === "k" || key === "ArrowUp") move(-1);
      else if (key === "Enter") {
        // Enter na fokusowanym linku/przycisku robi swoje.
        if (target?.closest?.("a, button") || activeRef.current === null) return;
        open?.(activeRef.current);
      } else if (key === "x" || key === "a" || key === "r") act?.(key, activeRef.current);
      else if (key === "Escape") {
        act?.("Escape", activeRef.current);
        set(null);
        return;
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled]);

  // Bieżący wiersz ma być widoczny po przesunięciu.
  useEffect(() => {
    document.querySelector(".kb-current")?.scrollIntoView?.({ block: "nearest" });
  }, [activeKey]);

  return { index, activeKey };
}
