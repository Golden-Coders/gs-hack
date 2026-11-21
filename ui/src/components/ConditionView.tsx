import { Code } from "@atlaskit/code";
import { Text } from "@atlaskit/primitives/compiled";
import type { Schemas } from "../api/client";
import "./ConditionView.css";

type Cond = Schemas["Condition"];

const OP_LABEL: Record<NonNullable<Cond["op"]>, string> = {
  eq: "=",
  ne: "≠",
  in: "in",
  gt: ">",
  gte: "≥",
  lt: "<",
  lte: "≤",
  contains: "contains",
  regex: "matches regex",
  exists: "exists",
  glob: "matches pattern",
};

const Op = ({ children }: { children: string }) => (
  <Text size="small" weight="semibold" color="color.text.subtle">
    {children.toUpperCase()}
  </Text>
);
const Muted = ({ children }: { children: string }) => <Text color="color.text.subtlest">{children}</Text>;

function fmt(value: unknown): string {
  return typeof value === "string" ? `"${value}"` : JSON.stringify(value);
}

function Node({ cond }: { cond: Cond }) {
  const items: React.ReactNode[] = [];
  if (cond.all) {
    items.push(
      <li key="all">
        <Op>all of</Op>
        <ul>
          {cond.all.length ? (
            cond.all.map((c, i) => <Node key={i} cond={c} />)
          ) : (
            <li>
              <Muted>(empty list = true)</Muted>
            </li>
          )}
        </ul>
      </li>,
    );
  }
  if (cond.any) {
    items.push(
      <li key="any">
        <Op>any of</Op>
        <ul>
          {cond.any.length ? (
            cond.any.map((c, i) => <Node key={i} cond={c} />)
          ) : (
            <li>
              <Muted>(empty list = false)</Muted>
            </li>
          )}
        </ul>
      </li>,
    );
  }
  if (cond.not) {
    items.push(
      <li key="not">
        <Op>not</Op>
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
        <Code>{cond.field}</Code>
        <Text color="color.text.subtle">{OP_LABEL[op] ?? op}</Text>
        {op !== "exists" && <Code>{fmt(cond.value)}</Code>}
      </li>,
    );
  }
  return <>{items}</>;
}

export default function ConditionView({ condition }: { condition?: Cond | null }) {
  const empty = !condition || (!condition.all && !condition.any && !condition.not && condition.field === undefined);
  if (empty) return <Muted>no condition (matches every alert)</Muted>;
  return (
    <div className="condition">
      <ul>
        <Node cond={condition} />
      </ul>
    </div>
  );
}
