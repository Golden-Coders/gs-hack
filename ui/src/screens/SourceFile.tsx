// Ścieżka pliku konfiguracyjnego, który trzeba edytować (UI jest tylko do odczytu), z przyciskiem kopiowania.
import { useState } from "react";
import "./SourceFile.css";

export function ReadOnlyNote({ children }: { children: React.ReactNode }) {
  return (
    <p className="readonly-note">
      <span className="readonly-chip">Tylko do odczytu</span>
      <span>{children}</span>
    </p>
  );
}

export default function SourceFile({ path, label = "Edytuj w:" }: { path?: string | null; label?: string }) {
  const [status, setStatus] = useState<"idle" | "copied" | "failed">("idle");

  if (!path) {
    return (
      <div className="source-file">
        <span className="source-file-label">{label}</span>
        <span className="source-file-missing">brak ścieżki pliku w odpowiedzi API</span>
      </div>
    );
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(path ?? "");
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
    window.setTimeout(() => setStatus("idle"), 2000);
  }

  return (
    <div className="source-file">
      <span className="source-file-label">{label}</span>
      <code>{path}</code>
      <button type="button" onClick={copy} aria-label={`Kopiuj ścieżkę ${path}`}>
        Kopiuj
      </button>
      <span role="status" className={status === "failed" ? "copy-status failed" : "copy-status"}>
        {status === "copied" ? "Skopiowano" : status === "failed" ? "Nie udało się skopiować" : ""}
      </span>
    </div>
  );
}
