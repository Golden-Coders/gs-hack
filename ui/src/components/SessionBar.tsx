// Przełącznik backendu (mock / prawdziwy) i wybór tożsamości („działam jako”).
import { useEffect, useState } from "react";
import { api, type Schemas } from "../api/client";
import { Stack, Text } from "@atlaskit/primitives/compiled";
import StreamIndicator from "./StreamIndicator";
import { Field, Select, TextInput } from "../ui/primitives";
import { API_URL, backend, setPersonId, switchBackend, usePersonId, type Backend } from "../api/session";

const BACKENDS: { value: Backend; label: string }[] = [
  { value: "mock", label: "mock (Prism)" },
  { value: "api", label: "prawdziwy (api)" },
];

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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- lista osób pobierana raz na start
  }, []);

  const personOptions = [
    ...(!personId ? [{ value: "", label: "—" }] : []),
    ...(personId && !people.some((p) => p.id === personId) ? [{ value: personId, label: personId }] : []),
    ...people.map((p) => ({ value: p.id, label: p.name })),
  ];

  return (
    <Stack space="space.100">
      {people.length ? (
        <Field label="Działam jako">
          {(f) => <Select {...f} value={personId ?? ""} options={personOptions} onChange={(v) => setPersonId(v || null)} />}
        </Field>
      ) : (
        // Lista osób niedostępna (np. backend bez /people) – identyfikator wpisany ręcznie.
        <Field label="Działam jako" hint="Lista osób niedostępna – wpisz identyfikator (np. anna).">
          {(f) => (
            <TextInput
              {...f}
              isCompact
              defaultValue={personId ?? ""}
              onBlur={(e) => setPersonId(e.target.value.trim() || null)}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
          )}
        </Field>
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
