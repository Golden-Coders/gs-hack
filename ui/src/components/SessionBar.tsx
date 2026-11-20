// Przełącznik backendu (mock / prawdziwy) i wybór tożsamości („działam jako”).
import { useEffect, useState } from "react";
import { api, type Schemas } from "../api/client";
import { API_URL, backend, setPersonId, switchBackend, usePersonId, type Backend } from "../api/session";

export default function SessionBar() {
  const personId = usePersonId();
  const [people, setPeople] = useState<Schemas["PersonDef"][]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.GET("/people").then(({ data, error }) => {
      if (error || !data) {
        setError("Nie udało się pobrać osób");
        return;
      }
      setPeople(data.people);
      // Pierwsze uruchomienie albo osoba zniknęła z konfiguracji → pierwsza z listy.
      if (data.people.length && !data.people.some((p) => p.id === personId)) {
        setPersonId(data.people[0].id);
      }
    }, () => setError(`Brak połączenia z ${API_URL}`));
  }, []); // lista osób pobierana raz na start

  return (
    <div className="session">
      <label>
        Działam jako
        <select value={personId ?? ""} onChange={(e) => setPersonId(e.target.value || null)}>
          {!personId && <option value="">—</option>}
          {personId && !people.some((p) => p.id === personId) && <option value={personId}>{personId}</option>}
          {people.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Backend
        <select value={backend} onChange={(e) => switchBackend(e.target.value as Backend)}>
          <option value="mock">mock (Prism)</option>
          <option value="api">prawdziwy (api)</option>
        </select>
      </label>
      <div className="api-url" title="Adres backendu">{API_URL}</div>
      {error && <div className="session-error">{error}</div>}
    </div>
  );
}
