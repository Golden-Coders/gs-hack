// Ekran Teams: przejrzysta lista zespołów po lewej stronie oraz prosty edytor zespołu po prawej,
// w którym dodajemy/usuwamy osoby z listy checkboxami.
import { useEffect, useState } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import { api, API_URL, type Schemas } from "../api/client";
import { describeError } from "../api/request";
import { getPersonId, getSessionToken } from "../api/session";
import { useStream } from "../api/stream";
import { useResource } from "../api/useResource";
import { Card, Checkbox, ConfirmInline, Empty, ErrorBox, Field, Loading, PageHeader, TextInput } from "../ui/primitives";
import { slugify } from "./rules/model";
import "./People.css";

type Person = Schemas["PersonDef"];
type Team = Schemas["TeamDef"];

export default function People() {
  const peopleRes = useResource(() => api.GET("/people"));
  const teamsRes = useResource(() => api.GET("/teams"));

  const people: Person[] = peopleRes.data?.people ?? [];
  const teams: Team[] = teamsRes.data ?? [];
  const failure = peopleRes.error ?? teamsRes.error;

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isCreatingNew, setIsCreatingNew] = useState(false);

  const refresh = () => {
    peopleRes.refresh();
    teamsRes.refresh();
  };

  useStream((msg) => {
    if (msg.type === "ui_event" && msg.data.kind === "config_changed") {
      refresh();
    }
  });

  useEffect(() => {
    if (selectedId !== null || isCreatingNew) return;
    if (teams.length > 0) {
      setSelectedId(teams[0].id);
    }
  }, [teams, selectedId, isCreatingNew]);

  const activeTeam = isCreatingNew
    ? null
    : selectedId
      ? (teams.find((t) => t.id === selectedId) ?? null)
      : null;

  const peopleById = new Map(people.map((p) => [p.id, p]));

  function handleSelect(t: Team) {
    setIsCreatingNew(false);
    setSelectedId(t.id);
  }

  function handleStartNew() {
    setSelectedId(null);
    setIsCreatingNew(true);
  }

  function handleSaved(saved: Team) {
    setIsCreatingNew(false);
    setSelectedId(saved.id);
    refresh();
  }

  async function handleDelete(id: string): Promise<string | null> {
    const sess = getSessionToken();
    const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
    const headers: Record<string, string> = { Authorization: `Bearer ${bearer}` };
    const personId = getPersonId();
    if (personId) headers["X-Person-Id"] = personId;

    const resp = await fetch(`${API_URL}/teams/${encodeURIComponent(id)}`, {
      method: "DELETE",
      credentials: "include",
      headers,
    });
    if (!resp.ok) {
      const errBody = await resp.json().catch(() => null);
      return describeError(errBody, resp);
    }
    const remaining = teams.filter((t) => t.id !== id);
    setSelectedId(remaining[0]?.id ?? null);
    setIsCreatingNew(false);
    refresh();
    return null;
  }

  return (
    <section className="tm-screen">
      <PageHeader
        actions={
          <Button appearance="primary" onClick={handleStartNew}>
            + Nowy team
          </Button>
        }
      >
        Teams
      </PageHeader>

      {failure && <ErrorBox message={failure} onRetry={refresh} />}

      <div className="tm-layout">
        {/* Lewa kolumna: lista zespołów */}
        <aside className="tm-sidebar" aria-label="Lista zespołów">
          {(peopleRes.loading || teamsRes.loading) && teamsRes.data === null && <Loading />}
          {!teamsRes.loading && teams.length === 0 && <Empty>Brak zespołów. Kliknij „+ Nowy team”.</Empty>}
          <ul className="tm-list">
            {teams.map((t) => {
              const active = !isCreatingNew && t.id === selectedId;
              const memberNames = t.members
                .map((id) => peopleById.get(id)?.name ?? id)
                .join(", ");
              return (
                <li key={t.id}>
                  <button
                    type="button"
                    className={`tm-item ${active ? "active" : ""}`}
                    onClick={() => handleSelect(t)}
                  >
                    <div className="tm-item-title">{t.name}</div>
                    <div className="tm-item-sub">
                      {t.members.length > 0 ? `Osoby (${t.members.length}): ${memberNames}` : "Brak przypisanych osób"}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        </aside>

        {/* Prawa kolumna: edytor zespołu i wybór osób z listy */}
        <div className="tm-main">
          {isCreatingNew || activeTeam ? (
            <TeamEditor
              key={isCreatingNew ? "__new__" : activeTeam?.id}
              initial={activeTeam}
              people={people}
              onSaved={handleSaved}
              onDelete={handleDelete}
            />
          ) : (
            <Card>
              <Heading size="small" as="h2">
                Wybierz team z listy po lewej stronie
              </Heading>
            </Card>
          )}
        </div>
      </div>
    </section>
  );
}

function TeamEditor({
  initial,
  people,
  onSaved,
  onDelete,
}: {
  initial: Team | null;
  people: Person[];
  onSaved: (team: Team) => void;
  onDelete: (id: string) => Promise<string | null>;
}) {
  const isNew = initial === null;
  const [name, setName] = useState(initial?.name ?? "");
  const [members, setMembers] = useState<string[]>(initial?.members ?? []);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function togglePerson(personId: string) {
    setMembers((prev) => (prev.includes(personId) ? prev.filter((id) => id !== personId) : [...prev, personId]));
  }

  async function handleSave() {
    setError(null);
    const cleanName = name.trim();
    if (!cleanName) {
      setError("Podaj nazwę zespołu.");
      return;
    }
    const id = isNew ? slugify(cleanName) || `team-${Date.now()}` : initial.id;
    const payload: Team = {
      id,
      name: cleanName,
      members,
      preferences: initial?.preferences ?? [],
    };

    setSaving(true);
    try {
      const sess = getSessionToken();
      const bearer = sess ?? (import.meta.env.VITE_API_TOKEN ?? "dev-token");
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
        Authorization: `Bearer ${bearer}`,
      };
      const personId = getPersonId();
      if (personId) headers["X-Person-Id"] = personId;

      const url = isNew ? `${API_URL}/teams` : `${API_URL}/teams/${encodeURIComponent(initial.id)}`;
      const method = isNew ? "POST" : "PUT";

      const resp = await fetch(url, {
        method,
        credentials: "include",
        headers,
        body: JSON.stringify(payload),
      });
      if (!resp.ok) {
        const errBody = await resp.json().catch(() => null);
        setError(describeError(errBody, resp));
        return;
      }
      const saved = (await resp.json()) as Team;
      onSaved(saved);
    } catch {
      setError("Nie udało się połączyć z API.");
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    if (!initial) return;
    setSaving(true);
    setError(null);
    const err = await onDelete(initial.id);
    if (err) setError(`Nie udało się usunąć zespołu: ${err}`);
    setSaving(false);
  }

  return (
    <div className="tm-editor">
      <header className="tm-editor-head">
        <Heading size="medium" as="h2">
          {isNew ? "Nowy team" : name || initial.name}
        </Heading>
      </header>

      <Card title="1. Nazwa zespołu">
        <Field label="Nazwa teamu">
          {(f) => (
            <TextInput
              {...f}
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
              placeholder="np. Team Payments"
            />
          )}
        </Field>
      </Card>

      <Card title="2. Osoby w zespole (zaznacz z listy)">
        {people.length === 0 ? (
          <Empty>Brak osób w systemie.</Empty>
        ) : (
          <div className="tm-people-list">
            {people.map((p) => {
              const checked = members.includes(p.id);
              const email = p.contacts?.email;
              const phone = p.contacts?.phone ?? p.contacts?.sms;
              const contactInfo = [email, phone].filter(Boolean).join(" · ");
              return (
                <div
                  key={p.id}
                  className={`tm-person-row ${checked ? "checked" : ""}`}
                >
                  <Checkbox
                    isChecked={checked}
                    onChange={() => togglePerson(p.id)}
                    label={p.name}
                  />
                  {contactInfo && (
                    <Text size="small" color="color.text.subtle">
                      {contactInfo}
                    </Text>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </Card>

      {error && <ErrorBox message={error} />}

      <div className="tm-savebar">
        <Button appearance="primary" isLoading={saving} isDisabled={saving} onClick={() => void handleSave()}>
          {isNew ? "Utwórz team" : "Zapisz zmiany"}
        </Button>
        {!isNew && (
          <ConfirmInline
            label="Usuń team"
            question={`Usunąć team „${initial.name}”?`}
            confirmLabel="Tak, usuń"
            busy={saving}
            onConfirm={() => void handleConfirmDelete()}
          />
        )}
      </div>
    </div>
  );
}
