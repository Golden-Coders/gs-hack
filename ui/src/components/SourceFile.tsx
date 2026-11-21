// Ścieżka pliku konfiguracyjnego, który trzeba edytować (UI jest tylko do odczytu), z przyciskiem kopiowania.
import { CopyButton } from "../ui/primitives";
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
  if (!path) {
    return (
      <div className="source-file">
        <span className="source-file-label">{label}</span>
        <span className="source-file-missing">brak ścieżki pliku w odpowiedzi API</span>
      </div>
    );
  }

  return (
    <div className="source-file">
      <span className="source-file-label">{label}</span>
      <code>{path}</code>
      <CopyButton text={path} />
    </div>
  );
}
