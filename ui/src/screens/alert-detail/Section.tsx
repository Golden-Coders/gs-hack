// Ramka sekcji z nagłówkiem, stanem ładowania i błędem widocznym przy samej sekcji.
import type { ReactNode } from "react";
import type { Resource } from "./util";

export default function Section({
  title,
  resource,
  children,
}: {
  title: string;
  resource: Resource<unknown>;
  children: ReactNode;
}) {
  return (
    <section className="ad-section" aria-busy={resource.loading}>
      <header className="ad-section-head">
        <h2>{title}</h2>
        {resource.loading && <span className="muted">ładowanie…</span>}
      </header>
      {resource.error && (
        <div className="error-box" role="alert">
          {resource.error}{" "}
          <button type="button" onClick={resource.reload} disabled={resource.loading}>
            Spróbuj ponownie
          </button>
        </div>
      )}
      {children}
    </section>
  );
}
