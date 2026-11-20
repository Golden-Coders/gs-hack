// Kreator drzewa warunków (tasks.md 15.1): grupy „wszystkie / dowolny / nie” i liście „pole – operator – wartość”.
// Edytowalny odpowiednik ConditionView; stan trzyma właściciel (RuleEditor), tu tylko edycja drzewa.
import { useEffect, useRef } from "react";
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
  const fid = `${node.uid}-field`;
  const vid = `${node.uid}-value`;
  return (
    <div className="cb-leaf">
      <div className="cb-cell cb-cell-field">
        <label htmlFor={fid}>Pole</label>
        <input
          id={fid}
          data-focus={node.uid}
          className="mono"
          list={ctx.fieldListId}
          value={node.field}
          onChange={(e) => set({ field: e.target.value })}
          placeholder="np. raw.payload.desk"
          autoComplete="off"
          spellCheck={false}
          aria-invalid={fieldErr ? true : undefined}
          aria-describedby={fieldErr ? `${fid}-err` : undefined}
        />
        {fieldErr && (
          <span id={`${fid}-err`} className="rules-field-error">
            {fieldErr}
          </span>
        )}
      </div>
      <div className="cb-cell cb-cell-op">
        <label htmlFor={`${node.uid}-op`}>Warunek</label>
        <select
          id={`${node.uid}-op`}
          value={node.op}
          onChange={(e) => set({ op: e.target.value as Op })}
        >
          {OPS.map((o) => (
            <option key={o.op} value={o.op}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
      {node.op !== "exists" && (
        <div className="cb-cell cb-cell-value">
          <label htmlFor={vid}>Wartość</label>
          <input
            id={vid}
            value={node.value}
            onChange={(e) => set({ value: e.target.value })}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={valueErr ? true : undefined}
            aria-describedby={`${vid}-hint`}
          />
          <span id={`${vid}-hint`} className={valueErr ? "rules-field-error" : "rules-field-hint"}>
            {valueErr ?? opMeta?.hint ?? ""}
          </span>
        </div>
      )}
      <button
        type="button"
        className="cb-remove"
        onClick={onRemove}
        aria-label={`Usuń warunek ${node.field || "(bez pola)"}`}
      >
        Usuń
      </button>
    </div>
  );
}

function AddBar({ uid, ctx, canNest = true }: { uid: string; ctx: Ctx; canNest?: boolean }) {
  return (
    <div className="cb-add">
      <button type="button" data-focus={`add:${uid}`} onClick={() => ctx.add(uid, newLeaf())}>
        + Dodaj warunek
      </button>
      {canNest && (
        <>
          <button type="button" onClick={() => ctx.add(uid, newGroup("all"))}>
            + Dodaj grupę
          </button>
          <button type="button" onClick={() => ctx.add(uid, newNot())}>
            + Dodaj „nie”
          </button>
        </>
      )}
    </div>
  );
}

function Group({ node, ctx, isRoot, onRemove }: { node: GroupNode; ctx: Ctx; isRoot?: boolean; onRemove?: () => void }) {
  const warn = ctx.errors.warnings[node.uid];
  const selId = `${node.uid}-mode`;
  return (
    <div className={`cb-group cb-${node.kind}`} role="group" aria-label={node.kind === "all" ? "Grupa: wszystkie warunki" : "Grupa: dowolny warunek"}>
      <div className="cb-group-head">
        <label htmlFor={selId} className="sr-only">
          Sposób łączenia warunków w grupie
        </label>
        <select
          id={selId}
          className="cb-mode"
          value={node.kind}
          onChange={(e) => ctx.change(node.uid, (n) => ({ ...(n as GroupNode), kind: e.target.value as "all" | "any" }))}
        >
          <option value="all">wszystkie warunki (I)</option>
          <option value="any">dowolny warunek (LUB)</option>
        </select>
        <span className="muted cb-group-hint">
          {node.kind === "all" ? "alert musi spełniać każdy z poniższych" : "wystarczy, że alert spełnia jeden z poniższych"}
        </span>
        {!isRoot && onRemove && (
          <button type="button" className="cb-remove" onClick={onRemove} aria-label="Usuń grupę">
            Usuń grupę
          </button>
        )}
      </div>
      {warn && <p className="rules-field-hint">{warn}</p>}
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
        <span className="cb-not-label">nie</span>
        <span className="muted cb-group-hint">poniższy warunek NIE może być spełniony</span>
        <button type="button" className="cb-remove" onClick={onRemove} aria-label="Usuń negację">
          Usuń „nie”
        </button>
      </div>
      {err && <p className="rules-field-error">{err}</p>}
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
