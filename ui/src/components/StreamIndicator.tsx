// Stan połączenia na żywo; trzyma subskrypcję, więc strumień działa przez całą sesję.
import { useStream, useStreamStatus, type StreamStatus } from "../api/stream";

const LABELS: Record<StreamStatus, string> = {
  idle: "rozłączono",
  connecting: "łączenie…",
  live: "na żywo",
  reconnecting: "ponowne łączenie…",
  unavailable: "na żywo niedostępne (mock)",
};

export default function StreamIndicator() {
  useStream(() => {});
  const status = useStreamStatus();
  return <div className={`stream-status stream-${status}`}>● {LABELS[status]}</div>;
}
