export const PALETTE_NAMES = ["red", "orange", "yellow", "green", "blue", "purple", "gray"] as const;
export type PaletteName = (typeof PALETTE_NAMES)[number];

export function paletteColor(name: string | null | undefined): { fg: string; bg: string } | null {
  const n = name?.toLowerCase();
  if (!n || !(PALETTE_NAMES as readonly string[]).includes(n)) return null;
  return { fg: `var(--${n})`, bg: `var(--${n}-bg)` };
}
