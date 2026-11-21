// Prymitywy UI wspólne dla wszystkich ekranów (bez wiedzy o domenie). Style: ui.css.
import { useId, useState, type ReactNode } from "react";

/** Ramka sekcji: nagłówek z akcjami, stan ładowania i błąd z ponowieniem widoczne przy samej sekcji. */
export function Card({
  title,
  actions,
  loading,
  error,
  onRetry,
  className,
  children,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <section className={className ? `card ${className}` : "card"} aria-busy={loading || undefined}>
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

export function ErrorBox({ message, onRetry, disabled }: { message: string; onRetry?: () => void; disabled?: boolean }) {
  return (
    <div className="error-box" role="alert">
      {message}
      {onRetry && (
        <button type="button" onClick={onRetry} disabled={disabled}>
          Spróbuj ponownie
        </button>
      )}
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

/** Zakładki przełączające widok w obrębie sekcji. */
export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  label,
}: {
  tabs: { value: T; label: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="tabs" role="tablist" aria-label={label}>
      {tabs.map((t) => (
        <button key={t.value} type="button" role="tab" aria-selected={t.value === value} onClick={() => onChange(t.value)}>
          {t.label}
        </button>
      ))}
    </div>
  );
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
}: {
  label: ReactNode;
  question: ReactNode;
  confirmLabel: ReactNode;
  onConfirm: () => void;
  busy?: boolean;
  disabled?: boolean;
}) {
  const [asking, setAsking] = useState(false);
  if (!asking)
    return (
      <button type="button" className="danger" onClick={() => setAsking(true)} disabled={disabled || busy}>
        {label}
      </button>
    );
  return (
    <span className="confirm-inline" role="group">
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
