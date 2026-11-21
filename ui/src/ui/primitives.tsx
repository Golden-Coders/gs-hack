// Prymitywy UI wspólne dla wszystkich ekranów (bez wiedzy o domenie). Style: ui.css.
import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

/** Ramka sekcji: nagłówek z akcjami, stan ładowania i błąd z ponowieniem widoczne przy samej sekcji. */
export function Card({
  title,
  actions,
  loading,
  error,
  onRetry,
  className,
  flush,
  children,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
  className?: string;
  /** Bez dolnego marginesu – dla kart w kontenerach flex/grid z własnym `gap`. */
  flush?: boolean;
  children?: ReactNode;
}) {
  const cls = ["card", flush && "card-flush", className].filter(Boolean).join(" ");
  return (
    <section className={cls} aria-busy={loading || undefined}>
      {(title || actions || loading) && (
        <header className="card-head">
          {title && <h2>{title}</h2>}
          {loading && <span className="muted">ładowanie…</span>}
          {actions && <div className="card-actions">{actions}</div>}
        </header>
      )}
      {error && <ErrorBox message={error} onRetry={onRetry} disabled={loading} />}
      {children}
    </section>
  );
}

export function ErrorBox({
  message,
  onRetry,
  disabled,
  id,
}: {
  message: ReactNode;
  onRetry?: () => void;
  disabled?: boolean;
  id?: string;
}) {
  return (
    <div className="error-box" role="alert" id={id}>
      {message}
      {onRetry && (
        <button type="button" onClick={onRetry} disabled={disabled}>
          Spróbuj ponownie
        </button>
      )}
    </div>
  );
}

/** Komunikat o powodzeniu (np. „Zapisano regułę”), ogłaszany czytnikom ekranu. */
export function Notice({ children }: { children: ReactNode }) {
  return (
    <div className="notice" role="status">
      {children}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="empty">{children}</p>;
}

export function Loading({ children = "Ładowanie…" }: { children?: ReactNode }) {
  return <p className="loading">{children}</p>;
}

/** Przełącznik kilku wartości (np. filtr „mój zespół / wszystkie”). */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="segmented" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} type="button" aria-pressed={o.value === value} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * Zakładki przełączające widok w obrębie sekcji. Strzałki/Home/End przenoszą fokus i wybór (roving tabindex);
 * z `idPrefix` zakładki dostają `id`/`aria-controls`, a panel – `tabPanelProps(idPrefix, value)`.
 */
export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  label,
  idPrefix,
}: {
  tabs: { value: T; label: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
  idPrefix?: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const i = tabs.findIndex((t) => t.value === value);
    const next =
      e.key === "ArrowRight" ? (i + 1) % tabs.length
      : e.key === "ArrowLeft" ? (i - 1 + tabs.length) % tabs.length
      : e.key === "Home" ? 0
      : e.key === "End" ? tabs.length - 1
      : -1;
    if (next < 0) return;
    e.preventDefault();
    onChange(tabs[next].value);
    refs.current[next]?.focus();
  }
  return (
    <div className="tabs" role="tablist" aria-label={label} onKeyDown={onKeyDown}>
      {tabs.map((t, i) => (
        <button
          key={t.value}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="tab"
          id={idPrefix ? `${idPrefix}-tab-${t.value}` : undefined}
          aria-controls={idPrefix ? `${idPrefix}-panel-${t.value}` : undefined}
          aria-selected={t.value === value}
          tabIndex={t.value === value ? 0 : -1}
          onClick={() => onChange(t.value)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

/** Atrybuty panelu powiązanego z `Tabs` o tym samym `idPrefix`. */
export function tabPanelProps(idPrefix: string, value: string) {
  return { role: "tabpanel", id: `${idPrefix}-panel-${value}`, "aria-labelledby": `${idPrefix}-tab-${value}`, tabIndex: 0 } as const;
}

/** Pole formularza: widoczna etykieta, podpowiedź i błąd powiązane z kontrolką przez id/aria. */
export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  children: (props: { id: string; "aria-describedby"?: string; "aria-invalid"?: boolean }) => ReactNode;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  return (
    <div className={error ? "field has-error" : "field"}>
      <label htmlFor={id}>{label}</label>
      {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
      {hint && (
        <div id={hintId} className="field-hint">
          {hint}
        </div>
      )}
      {error && (
        <div id={errorId} className="field-error">
          {error}
        </div>
      )}
    </div>
  );
}

/** Akcja wymagająca potwierdzenia w miejscu (bez okna dialogowego): „Usuń” → „Tak, usuń / Anuluj”. */
export function ConfirmInline({
  label,
  question,
  confirmLabel,
  onConfirm,
  busy,
  disabled,
  ariaLabel,
  onAskingChange,
}: {
  label: ReactNode;
  question: ReactNode;
  confirmLabel: ReactNode;
  onConfirm: () => void;
  busy?: boolean;
  disabled?: boolean;
  /** Pełna nazwa akcji dla czytników, np. „Usuń mapping workflow.v1”. */
  ariaLabel?: string;
  /** Informuje rodzica, że trwa pytanie (np. żeby schować sąsiednie przyciski). */
  onAskingChange?: (asking: boolean) => void;
}) {
  const [asking, setAskingState] = useState(false);
  const setAsking = (v: boolean) => {
    setAskingState(v);
    onAskingChange?.(v);
  };
  if (!asking)
    return (
      <button type="button" className="danger" aria-label={ariaLabel} onClick={() => setAsking(true)} disabled={disabled || busy}>
        {busy ? "Trwa…" : label}
      </button>
    );
  return (
    <span className="confirm-inline" role="group" aria-label={ariaLabel}>
      <span>{question}</span>
      <button
        type="button"
        className="danger"
        disabled={busy}
        onClick={() => {
          onConfirm();
          setAsking(false);
        }}
      >
        {confirmLabel}
      </button>
      <button type="button" onClick={() => setAsking(false)} disabled={busy} autoFocus>
        Anuluj
      </button>
    </span>
  );
}

/** Kopiowanie tekstu do schowka z informacją o wyniku dla czytników ekranu. */
export function CopyButton({ text, label = "Kopiuj" }: { text: string; label?: string }) {
  const [status, setStatus] = useState<"idle" | "ok" | "failed">("idle");
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setStatus("ok");
    } catch {
      setStatus("failed");
    }
    setTimeout(() => setStatus("idle"), 2000);
  }
  return (
    <>
      <button type="button" onClick={copy} aria-label={`${label}: ${text}`}>
        {label}
      </button>
      <span role="status" className={status === "failed" ? "copy-status failed" : "copy-status"}>
        {status === "ok" ? "Skopiowano" : status === "failed" ? "Nie udało się skopiować" : ""}
      </span>
    </>
  );
}
