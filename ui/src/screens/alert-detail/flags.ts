// Wynik akcji użytkownika jako Flag (spec „Powiadomienie o wyniku akcji”): sukces znika sam, błąd zostaje do zamknięcia.
import { useFlags } from "@atlaskit/flag";

export function useResultFlag() {
  const { showFlag } = useFlags();
  return (err: string | null | undefined, ok: string, failed: string) =>
    err
      ? showFlag({ title: failed, description: err, appearance: "error" })
      : showFlag({ title: ok, appearance: "success", isAutoDismiss: true });
}
