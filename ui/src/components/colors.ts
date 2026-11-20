// Kolory z prezentacji reguły (`color`, `highlight`) to nazwy; mapujemy je na stałą paletę,
// a nieznane nazwy ignorujemy (konfiguracja nie wstrzykuje dowolnego CSS).
const PALETTE: Record<string, { fg: string; bg: string }> = {
  red: { fg: "#b91c1c", bg: "#fee2e2" },
  orange: { fg: "#c2410c", bg: "#ffedd5" },
  yellow: { fg: "#a16207", bg: "#fef9c3" },
  green: { fg: "#15803d", bg: "#dcfce7" },
  blue: { fg: "#1d4ed8", bg: "#dbeafe" },
  purple: { fg: "#7e22ce", bg: "#f3e8ff" },
  gray: { fg: "#4b5563", bg: "#f3f4f6" },
};

export function paletteColor(name: string | null | undefined): { fg: string; bg: string } | null {
  return (name && PALETTE[name.toLowerCase()]) || null;
}
