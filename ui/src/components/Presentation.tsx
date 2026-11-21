// Alert w wyglądzie wg prezentacji dopasowanej reguły: tytuł, treść, wybrane pola z wyróżnieniem, kolor.
// Szablony renderuje backend (`condition`), tu tylko pokazujemy wynik z `AlertDetail`.
import type { Schemas } from "../api/client";
import { SeverityBadge, StateBadge } from "./Badges";
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
    <dl className="presented-fields">
      {fields.map((f) => {
        const color = paletteColor(f.highlight);
        return (
          <div
            key={f.path}
            className={color ? "presented-field highlighted" : "presented-field"}
            style={color ? { color: color.fg, background: color.bg } : undefined}
          >
            <dt title={f.path}>{f.label}</dt>
            <dd>{formatValue(f.value)}</dd>
          </div>
        );
      })}
    </dl>
  );
}

export default function Presentation({ alert }: { alert: Schemas["AlertDetail"] }) {
  const color = paletteColor(alert.color);
  return (
    <article className="presentation" style={color ? { borderLeftColor: color.fg } : undefined}>
      <header>
        <SeverityBadge severity={alert.severity} />
        <StateBadge state={alert.state} />
        <h2 style={color ? { color: color.fg } : undefined}>{alert.title}</h2>
      </header>
      {alert.body && <p className="presentation-body">{alert.body}</p>}
      <PresentedFields fields={alert.fields} />
    </article>
  );
}
