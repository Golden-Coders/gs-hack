// Przełącznik backendu (mock / prawdziwy) i uwierzytelniona sesja operatora („działam jako”).
import { useEffect, useState } from "react";
import { api, type Schemas } from "../api/client";
import { Stack, Text } from "@atlaskit/primitives/compiled";
import StreamIndicator from "./StreamIndicator";
import { Field, Select } from "../ui/primitives";
import { API_URL, backend, loginAs, logout, switchBackend, usePersonId, type Backend } from "../api/session";

const BACKENDS: { value: Backend; label: string }[] = [
  { value: "mock", label: "mock (Prism)" },
  { value: "api", label: "prawdziwy (api)" },
];

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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- lista osób pobierana raz na start
  }, []);

  const personOptions = [
    ...(!personId ? [{ value: "", label: "— nie zalogowano —" }] : []),
    ...people.map((p) => ({ value: p.id, label: p.name })),
  ];

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
    <Stack space="space.100">
      <Field label="Działam jako">
        {(f) => (
          <Select
            {...f}
            value={personId ?? ""}
            options={personOptions}
            onChange={(v) => void handleSelect(v)}
          />
        )}
      </Field>
      {personId && (
        <button type="button" onClick={() => void logout()}>
          Wyloguj
        </button>
      )}
      <Field label="Backend">
        {(f) => <Select {...f} value={backend} options={BACKENDS} onChange={(v) => switchBackend(v as Backend)} />}
      </Field>
      <StreamIndicator />
      <span style={{ wordBreak: "break-all" }}>
        <Text size="small" color="color.text.subtlest">
          Adres backendu: {API_URL}
        </Text>
      </span>
      {error && (
        <Text size="small" color="color.text.danger">
          {error}
        </Text>
      )}
    </Stack>
  );
}
