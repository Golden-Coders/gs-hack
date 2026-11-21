// Przełącznik backendu (mock / prawdziwy) i uwierzytelniona sesja operatora („działam jako”).
import { useEffect, useState } from "react";
import { api, type Schemas } from "../api/client";
import StreamIndicator from "./StreamIndicator";
import { API_URL, backend, loginAs, logout, switchBackend, usePersonId, type Backend } from "../api/session";

export default function SessionBar() {
  const personId = usePersonId();
  const [people, setPeople] = useState<Schemas["PersonDef"][]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.GET("/people").then(
      ({ data, error }) => {
        if (error || !data) {
          setError("Nie udało się pobrać osób");
          return;
        }
        setPeople(data.people);
        if (data.people.length && !data.people.some((p) => p.id === personId)) {
          void loginAs(data.people[0].id).catch((e: unknown) =>
            setError(e instanceof Error ? e.message : "Błąd logowania"),
          );
        }
      },
      () => setError(`Brak połączenia z ${API_URL}`),
    );
  }, []); // lista osób pobierana raz na start

  async function handleSelect(nextId: string) {
    setError(null);
    if (!nextId) {
      await logout();
      return;
    }
    try {
      await loginAs(nextId);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Błąd logowania");
    }
  }

  return (
    <div className="session">
      <label>
        Działam jako
        <select value={personId ?? ""} onChange={(e) => void handleSelect(e.target.value)} disabled={!people.length}>
          {!personId && <option value="">— nie zalogowano —</option>}
          {people.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      {personId && (
        <button type="button" onClick={() => void logout()}>
          Wyloguj
        </button>
      )}
      <label>
        Backend
        <select value={backend} onChange={(e) => switchBackend(e.target.value as Backend)}>
          <option value="mock">mock (Prism)</option>
          <option value="api">prawdziwy (api)</option>
        </select>
      </label>
      <StreamIndicator />
      <div className="api-url" title="Adres backendu">{API_URL}</div>
      {error && <div className="session-error">{error}</div>}
    </div>
  );
}
