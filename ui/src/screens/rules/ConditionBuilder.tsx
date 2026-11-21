// Kreator drzewa warunków (tasks.md 15.1): grupy „wszystkie / dowolny / nie” i liście „pole – operator – wartość”.
// Edytowalny odpowiednik ConditionView; stan trzyma właściciel (RuleEditor), tu tylko edycja drzewa.
import { useEffect, useRef } from "react";
import Button from "@atlaskit/button/new";
import { Text } from "@atlaskit/primitives/compiled";
import { Field, Select, TextInput } from "../../ui/primitives";
import {
  OPS,
  mapTree,
  newGroup,
  newLeaf,
  newNot,
  type FormErrors,
  type GroupNode,
  type LeafNode,
  type NotNode,
  type Op,
  type TreeNode,
} from "./model";

interface Ctx {
  errors: FormErrors;
  showRequired: boolean;
  fieldListId: string;
  change: (uid: string, fn: (n: TreeNode) => TreeNode | null, focus?: string) => void;
  /** Dodaje węzeł do grupy / negacji i przenosi na niego fokus. */
  add: (parentUid: string, node: TreeNode) => void;
}

function Leaf({ node, ctx, onRemove }: { node: LeafNode; ctx: Ctx; onRemove: () => void }) {
  const err = ctx.errors.leaves[node.uid];
  const fieldErr = err?.field && (ctx.showRequired || err.kind === "format") ? err.field : undefined;
  const valueErr = err?.value && (ctx.showRequired || err.kind === "format") ? err.value : undefined;
  const opMeta = OPS.find((o) => o.op === node.op);
  const set = (patch: Partial<LeafNode>) => ctx.change(node.uid, (n) => ({ ...(n as LeafNode), ...patch }));
  return (
    <div className="cb-leaf">
      <Field label="Pole" error={fieldErr}>
        {(f) => (
          <TextInput
            {...f}
            data-focus={node.uid}
            isMonospaced
            list={ctx.fieldListId}
            value={node.field}
            onChange={(e) => set({ field: e.currentTarget.value })}
            placeholder="np. raw.payload.desk"
            autoComplete="off"
            spellCheck={false}
          />
        )}
      </Field>
      <Field label="Warunek">
        {(f) => (
          <Select {...f} value={node.op} onChange={(op) => set({ op: op as Op })} options={OPS.map((o) => ({ value: o.op, label: o.label }))} />
        )}
      </Field>
      {node.op !== "exists" && (
        <Field label="Wartość" hint={valueErr ? undefined : opMeta?.hint} error={valueErr}>
          {(f) => <TextInput {...f} value={node.value} onChange={(e) => set({ value: e.currentTarget.value })} autoComplete="off" spellCheck={false} />}
        </Field>
      )}
      <div className="cb-remove">
        <Button onClick={onRemove} aria-label={`Usuń warunek ${node.field || "(bez pola)"}`}>
          Usuń
        </Button>
      </div>
    </div>
  );
}

function AddBar({ uid, ctx, canNest = true }: { uid: string; ctx: Ctx; canNest?: boolean }) {
  return (
    <div className="cb-add">
      <Button data-focus={`add:${uid}`} onClick={() => ctx.add(uid, newLeaf())}>
        Dodaj warunek
      </Button>
      {canNest && (
        <>
          <Button onClick={() => ctx.add(uid, newGroup("all"))}>Dodaj grupę</Button>
          <Button onClick={() => ctx.add(uid, newNot())}>Dodaj „nie”</Button>
        </>
      )}
    </div>
  );
}

function Group({ node, ctx, isRoot, onRemove }: { node: GroupNode; ctx: Ctx; isRoot?: boolean; onRemove?: () => void }) {
  const warn = ctx.errors.warnings[node.uid];
  return (
    <div className={`cb-group cb-${node.kind}`} role="group" aria-label={node.kind === "all" ? "Grupa: wszystkie warunki" : "Grupa: dowolny warunek"}>
      <div className="cb-group-head">
        <div className="cb-mode">
          <Select
            aria-label="Sposób łączenia warunków w grupie"
            value={node.kind}
            onChange={(kind) => ctx.change(node.uid, (n) => ({ ...(n as GroupNode), kind: kind as "all" | "any" }))}
            options={[
              { value: "all", label: "wszystkie warunki (I)" },
              { value: "any", label: "dowolny warunek (LUB)" },
            ]}
          />
        </div>
        <span className="cb-group-hint">
          <Text size="small" color="color.text.subtle">
            {node.kind === "all" ? "alert musi spełniać każdy z poniższych" : "wystarczy, że alert spełnia jeden z poniższych"}
          </Text>
        </span>
        {!isRoot && onRemove && (
          <Button onClick={onRemove} aria-label="Usuń grupę">
            Usuń grupę
          </Button>
        )}
      </div>
      {warn && (
        <Text as="p" size="small" color="color.text.subtle">
          {warn}
        </Text>
      )}
      <ul className="cb-children">
        {node.children.map((c) => (
          <li key={c.uid}>
            <Node
              node={c}
              ctx={ctx}
              onRemove={() => ctx.change(c.uid, () => null, `add:${node.uid}`)}
            />
          </li>
        ))}
      </ul>
      <AddBar uid={node.uid} ctx={ctx} />
    </div>
  );
}

function Not({ node, ctx, onRemove }: { node: NotNode; ctx: Ctx; onRemove: () => void }) {
  const err = ctx.errors.groups[node.uid];
  return (
    <div className="cb-group cb-not" role="group" aria-label="Negacja: nie">
      <div className="cb-group-head">
        <span className="cb-not-label">
          <Text weight="bold" color="color.text.danger">
            NIE
          </Text>
        </span>
        <span className="cb-group-hint">
          <Text size="small" color="color.text.subtle">
            poniższy warunek NIE może być spełniony
          </Text>
        </span>
        <Button onClick={onRemove} aria-label="Usuń negację">
          Usuń „nie”
        </Button>
      </div>
      {err && (
        <Text as="p" color="color.text.danger">
          {err}
        </Text>
      )}
      {node.child ? (
        <Node
          node={node.child}
          ctx={ctx}
          onRemove={() => ctx.change(node.uid, (n) => ({ ...(n as NotNode), child: null }), `add:${node.uid}`)}
        />
      ) : (
        <AddBar uid={node.uid} ctx={ctx} />
      )}
    </div>
  );
}

function Node({ node, ctx, onRemove }: { node: TreeNode; ctx: Ctx; onRemove: () => void }) {
  if (node.kind === "leaf") return <Leaf node={node} ctx={ctx} onRemove={onRemove} />;
  if (node.kind === "not") return <Not node={node} ctx={ctx} onRemove={onRemove} />;
  return <Group node={node} ctx={ctx} onRemove={onRemove} />;
}

interface Props {
  root: GroupNode;
  onChange: (root: GroupNode) => void;
  errors: FormErrors;
  showRequired: boolean;
  fieldListId: string;
}

export default function ConditionBuilder({ root, onChange, errors, showRequired, fieldListId }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const pendingFocus = useRef<string | null>(null);

  useEffect(() => {
    const key = pendingFocus.current;
    if (!key) return;
    pendingFocus.current = null;
    container.current?.querySelector<HTMLElement>(`[data-focus="${key}"]`)?.focus();
  }, [root]);

  const ctx: Ctx = {
    errors,
    showRequired,
    fieldListId,
    change: (uid, fn, focus) => {
      if (focus) pendingFocus.current = focus;
      const next = mapTree(root, uid, fn);
      if (next && next.kind !== "leaf" && next.kind !== "not") onChange(next);
    },
    add: (parentUid, node) => {
      pendingFocus.current = node.kind === "leaf" ? node.uid : node.kind === "not" && node.child ? node.child.uid : `add:${node.uid}`;
      const next = mapTree(root, parentUid, (p) => {
        if (p.kind === "not") return { ...p, child: node };
        if (p.kind === "leaf") return p;
        return { ...p, children: [...p.children, node] };
      });
      if (next && next.kind !== "leaf" && next.kind !== "not") onChange(next);
    },
  };

  return (
    <div className="cb" ref={container}>
      <Group node={root} ctx={ctx} isRoot />
    </div>
  );
}
