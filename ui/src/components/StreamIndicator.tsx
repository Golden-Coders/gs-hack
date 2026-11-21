// Stan połączenia na żywo; trzyma subskrypcję, więc strumień działa przez całą sesję.
import { useStream, useStreamStatus } from "../api/stream";
import { StatusLozenge } from "../ui/primitives";

export default function StreamIndicator() {
  useStream(() => {});
  const status = useStreamStatus();
  return (
    <div role="status">
      <StatusLozenge domain="stream" value={status} />
    </div>
  );
}
