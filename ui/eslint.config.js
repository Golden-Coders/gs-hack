// ESLint 9 (flat config) — gwarancje design.md D7: komponenty i tokeny ADS, ikony tylko z lucide-react.
import designSystem from "@atlaskit/eslint-plugin-design-system";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

const restrictedImports = [
  { group: ["@atlaskit/icon", "@atlaskit/icon/*", "@atlaskit/icon-lab", "@atlaskit/icon-lab/*"], message: "Ikony tylko z lucide-react (licencja ADG) — użyj adsIcon() z src/ui/icons.tsx." },
  { group: ["@atlaskit/navigation-system", "@atlaskit/navigation-system/*"], message: "Peer tylko React 18.2 i decyzja D1: powłoka na @atlaskit/page-layout + @atlaskit/side-navigation." },
  { group: ["@atlassian/*"], message: "Pakiety @atlassian/* nie są publiczne." },
  { group: ["@atlaskit/css", "@atlaskit/css/*"], message: "Bez @atlaskit/css (wymaga transformacji Compiled, D3) — tokeny przez token() / var(--ds-*)." },
];

export default tseslint.config(
  { ignores: ["dist/**", "src/api/schema.d.ts"] },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { parser: tseslint.parser, parserOptions: { ecmaFeatures: { jsx: true } } },
    plugins: { "@typescript-eslint": tseslint.plugin, "react-hooks": reactHooks },
    rules: { "react-hooks/rules-of-hooks": "error", "react-hooks/exhaustive-deps": "warn" },
  },
  { files: ["src/**/*.{ts,tsx}"], ...designSystem.configs["recommended/flat"] },
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: { "no-restricted-imports": ["error", { patterns: restrictedImports }] },
  },
  // Faza 2: ekrany jeszcze używają `<Badge tone="gray">` (nazwy kolorów w propsie, fałszywy alarm reguły) —
  // ostrzeżenie zamiast błędu; zadanie 3.5 usuwa ten blok (i tak `--max-warnings 0`).
  { files: ["src/screens/**/*.{ts,tsx}"], rules: { "@atlaskit/design-system/ensure-design-token-usage": "warn" } },
);
