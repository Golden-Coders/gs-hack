// Wynik akcji użytkownika: nie pokazujemy zielonych powiadomień sukcesu, tylko błędy gdy coś pójdzie nie tak.
import { useFlags } from "@atlaskit/flag";

export function useResultFlag() {
  const { showFlag } = useFlags();
  return (err: string | null | undefined, _ok: string, failed: string) => {
    if (err) {
      showFlag({ title: failed, description: err, appearance: "error" });
    }
  };
}
