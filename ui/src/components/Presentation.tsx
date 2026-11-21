// Alert w wyglądzie wg prezentacji dopasowanej reguły: tytuł, treść, wybrane pola z wyróżnieniem, kolor.
// Szablony renderuje backend (`condition`), tu tylko pokazujemy wynik z `AlertDetail`.
// Kolor z reguły (paleta → tokeny akcentowe ADS) to wyjątek od reguły czerwieni: nadaje go konfiguracja.
import Heading from "@atlaskit/heading";
import { Inline, Stack, Text } from "@atlaskit/primitives/compiled";
import { token } from "@atlaskit/tokens";
import type { Schemas } from "../api/client";
import { SeverityBadge } from "./Badges";
import { paletteColor } from "./colors";

export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

export function PresentedFields({ fields }: { fields: Schemas["PresentedField"][] }) {
  if (!fields.length) return null;
  return (
    <dl
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
        gap: token("space.075"),
        margin: `${token("space.100")} 0 0`,
      }}
    >
      {fields.map((f) => {
        const color = paletteColor(f.highlight);
        return (
          <div
            key={f.path}
            style={{
              padding: `${token("space.075")} ${token("space.100")}`,
              borderRadius: token("radius.medium"),
              background: color?.bg ?? token("elevation.surface.sunken"),
              color: color?.fg,
              fontWeight: color ? token("font.weight.semibold") : undefined,
            }}
          >
            <dt title={f.path} style={{ font: token("font.body.small"), color: color ? "inherit" : token("color.text.subtle") }}>
              {f.label}
            </dt>
            <dd style={{ margin: 0, wordBreak: "break-word" }}>{formatValue(f.value)}</dd>
          </div>
        );
      })}
    </dl>
  );
}

export default function Presentation({ alert }: { alert: Schemas["AlertDetail"] }) {
  const color = paletteColor(alert.color);
  return (
    <article
      style={{
        background: token("elevation.surface.raised"),
        border: `${token("border.width")} solid ${token("color.border")}`,
        borderLeft: `${token("border.width.selected")} solid ${color?.fg ?? token("color.border.bold")}`,
        borderRadius: token("radius.large"),
        padding: `${token("space.150")} ${token("space.200")}`,
        marginBottom: token("space.150"),
      }}
    >
      <Stack space="space.100">
        <Inline space="space.100" alignBlock="center" shouldWrap>
          <SeverityBadge severity={alert.severity} />
          <Heading size="small" as="h2">
            <span style={color ? { color: color.fg } : undefined}>{alert.title}</span>
          </Heading>
        </Inline>
        {alert.body && <Text as="p">{alert.body}</Text>}
        <PresentedFields fields={alert.fields} />
      </Stack>
    </article>
  );
}
