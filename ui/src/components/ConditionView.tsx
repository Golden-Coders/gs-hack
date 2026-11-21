// Czytelny widok drzewa warunków: all / any / not z liśćmi `field op value`. Tylko do odczytu,
// użyteczny też w edytorze reguł. Ewaluacją zajmuje się backend (crate `condition`).
import type { Schemas } from "../api/client";
import "./ConditionView.css";

type Cond = Schemas["Condition"];

const OP_LABEL: Record<NonNullable<Cond["op"]>, string> = {
  eq: "=",
  ne: "≠",
  in: "należy do",
  gt: ">",
  gte: "≥",
  lt: "<",
  lte: "≤",
  contains: "zawiera",
  regex: "pasuje do regex",
  exists: "istnieje",
  glob: "pasuje do wzorca",
};

function fmt(value: unknown): string {
  return typeof value === "string" ? `"${value}"` : JSON.stringify(value);
}

function Node({ cond }: { cond: Cond }) {
  const items: React.ReactNode[] = [];
  if (cond.all) {
    items.push(
      <li key="all">
        <span className="condition-op">wszystkie z</span>
        <ul>
          {cond.all.length ? (
            cond.all.map((c, i) => <Node key={i} cond={c} />)
          ) : (
            <li className="condition-empty">(pusta lista = prawda)</li>
          )}
        </ul>
      </li>,
    );
  }
  if (cond.any) {
    items.push(
      <li key="any">
        <span className="condition-op">dowolny z</span>
        <ul>
          {cond.any.length ? (
            cond.any.map((c, i) => <Node key={i} cond={c} />)
          ) : (
            <li className="condition-empty">(pusta lista = fałsz)</li>
          )}
        </ul>
      </li>,
    );
  }
  if (cond.not) {
    items.push(
      <li key="not">
        <span className="condition-op not">nie</span>
        <ul>
          <Node cond={cond.not} />
        </ul>
      </li>,
    );
  }
  if (cond.field !== undefined) {
    const op = cond.op ?? "eq";
    items.push(
      <li key="leaf" className="condition-leaf">
        <span className="condition-field">{cond.field}</span>
        <span className="condition-cmp">{OP_LABEL[op] ?? op}</span>
        {op !== "exists" && <span className="condition-value">{fmt(cond.value)}</span>}
      </li>,
    );
  }
  return <>{items}</>;
}

export default function ConditionView({ condition }: { condition?: Cond | null }) {
  const empty = !condition || (!condition.all && !condition.any && !condition.not && condition.field === undefined);
  if (empty) return <span className="condition-empty">brak warunku (pasuje do każdego alertu)</span>;
  return (
    <div className="condition">
      <ul>
        <Node cond={condition} />
      </ul>
    </div>
  );
}
