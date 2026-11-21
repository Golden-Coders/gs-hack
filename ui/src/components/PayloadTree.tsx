import { useState } from "react";
import Button from "@atlaskit/button/new";
import { Pressable, Text } from "@atlaskit/primitives/compiled";
import { pathKey, type PathSeg } from "../lib/paths";
import "./PayloadTree.css";

export type PayloadTreeProps = {
  value: unknown;
  onSelect: (path: PathSeg[], value: unknown) => void;
  /** Oznaczenia pól: pathKey(ścieżka) -> etykiety (np. „fingerprint”, „tytuł”). */
  marks?: ReadonlyMap<string, string[]>;
  /** Opis akcji kliknięcia dla czytników ekranu, np. „Dodaj do fingerprintu”. */
  actionLabel?: string;
  /** Czy klik w obiekt/tablicę też wybiera (domyślnie tylko liście). */
  selectContainers?: boolean;
};

const PAGE = 25;

function preview(v: unknown): string {
  if (typeof v === "string") return `"${v}"`;
  return JSON.stringify(v);
}

type NodeProps = Required<Pick<PayloadTreeProps, "onSelect">> &
  Pick<PayloadTreeProps, "marks" | "actionLabel" | "selectContainers"> & {
    name: PathSeg;
    value: unknown;
    path: PathSeg[];
    depth: number;
  };

function Marks({ labels }: { labels?: string[] }) {
  if (!labels?.length) return null;
  return (
    <span className="ptree-marks">
      {labels.map((l) => (
        <span key={l} className="ptree-mark">
          {l}
        </span>
      ))}
    </span>
  );
}

function Node(p: NodeProps) {
  const { name, value, path, depth, marks, onSelect, actionLabel, selectContainers } = p;
  const [limit, setLimit] = useState(PAGE);
  const isContainer = value !== null && typeof value === "object";
  const labels = marks?.get(pathKey(path));
  const display = typeof name === "number" ? `[${name}]` : name;

  if (!isContainer) {
    return (
      <li className={`ptree-leaf${labels?.length ? " used" : ""}`}>
        <Pressable
          onClick={() => onSelect(path, value)}
          aria-label={`${actionLabel ?? "Wybierz"}: ${path.join(".")}, wartość ${preview(value)}`}
        >
          <span className="ptree-key">{display}</span>
          <span className={`ptree-val ptree-${value === null ? "null" : typeof value}`} title={String(value)}>
            {preview(value)}
          </span>
          <Marks labels={labels} />
        </Pressable>
      </li>
    );
  }

  const entries: [PathSeg, unknown][] = Array.isArray(value)
    ? value.map((v, i) => [i, v])
    : Object.entries(value as Record<string, unknown>);
  const shown = entries.slice(0, limit);
  return (
    <li>
      <details open={depth < 3}>
        <summary>
          <span className="ptree-key">{display}</span>
          <span className="ptree-meta">{Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`}</span>
          {selectContainers && (
            <Button
              spacing="compact"
              onClick={(e) => {
                e.preventDefault();
                onSelect(path, value);
              }}
            >
              Wybierz
            </Button>
          )}
          <Marks labels={labels} />
        </summary>
        <ul>
          {shown.map(([k, v]) => (
            <Node key={String(k)} {...p} name={k} value={v} path={[...path, k]} depth={depth + 1} />
          ))}
          {entries.length > shown.length && (
            <li>
              <Button spacing="compact" appearance="subtle" onClick={() => setLimit(limit + PAGE)}>
                Pokaż więcej (jeszcze {entries.length - shown.length})
              </Button>
            </li>
          )}
        </ul>
      </details>
    </li>
  );
}

export default function PayloadTree({ value, onSelect, marks, actionLabel, selectContainers }: PayloadTreeProps) {
  if (value === null || typeof value !== "object") {
    return (
      <Text as="p" color="color.text.subtle">
        Payload nie jest obiektem ani tablicą – nie ma pól do wyboru.
      </Text>
    );
  }
  const entries: [PathSeg, unknown][] = Array.isArray(value)
    ? value.map((v, i) => [i, v])
    : Object.entries(value as Record<string, unknown>);
  return (
    <div className="ptree">
      <ul>
        {entries.map(([k, v]) => (
          <Node
            key={String(k)}
            name={k}
            value={v}
            path={[k]}
            depth={1}
            marks={marks}
            onSelect={onSelect}
            actionLabel={actionLabel}
            selectContainers={selectContainers}
          />
        ))}
      </ul>
    </div>
  );
}
