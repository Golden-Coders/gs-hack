// Osoby, zespoły i preferencje powiadomień (tylko do odczytu) – tasks.md 15.4.
// Reguła wskazuje KTO, a JAK (kanały) wynika z preferencji osoby; zespół daje domyślne preferencje.
import { api, type Schemas } from "../api/client";
import { useResource } from "../api/useResource";
import DynamicTable from "@atlaskit/dynamic-table";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import Lozenge from "@atlaskit/lozenge";
import Tooltip from "@atlaskit/tooltip";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
import { Card, Empty, ErrorBox, Loading, PageHeader, StatusLozenge } from "../ui/primitives";
import "./People.css";

type Person = Schemas["PersonDef"];
type Team = Schemas["TeamDef"];
type Preference = Schemas["Preference"];
type Channel = Schemas["Channel"];

const SEVERITY_ORDER: Record<Schemas["Severity"], number> = { critical: 0, error: 1, warning: 2, info: 3 };

function Channels({ channels }: { channels: Channel[] }) {
  return (
    <>
      {channels.map((c, i) => (
        <span key={`${c}-${i}`}>
          {i > 0 && <span aria-hidden="true"><Text color="color.text.subtle">{" → "}</Text></span>}
          <StatusLozenge domain="channel" value={c} />
        </span>
      ))}
    </>
  );
}

function Preferences({ prefs, emptyText }: { prefs?: Preference[]; emptyText: string }) {
  if (!prefs || prefs.length === 0) return <Text color="color.text.subtle">{emptyText}</Text>;
  const sorted = [...prefs].sort((a, b) => SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity]);
  return (
    <ul className="prefs">
      {sorted.map((p, i) => (
        <li key={i} className="pref">
          <StatusLozenge domain="severity" value={p.severity} />
          <span aria-hidden="true"><Text color="color.text.subtle">
            →
          </Text></span>
          <Channels channels={p.channels} />
          <span className="pref-hours">{p.hours ? `godz. ${p.hours}` : "całą dobę"}</span>
        </li>
      ))}
    </ul>
  );
}

function personCells(person: Person) {
  const contacts = Object.entries(person.contacts ?? {});
  const fallback = person.fallback ?? [];
  return [
    {
      key: "person",
      content: (
        <>
          <div className="person-name">{person.name}</div>
          <div className="person-id">{person.id}</div>
        </>
      ),
    },
    { key: "tz", content: person.timezone ?? <Text color="color.text.subtle">—</Text> },
    {
      key: "contacts",
      content: contacts.length ? (
        <ul className="contacts">
          {contacts.map(([channel, value]) => (
            <li key={channel}>
              <span className="contact-channel">{channel}</span>
              <span className="mono">{value}</span>
            </li>
          ))}
        </ul>
      ) : (
        <Text color="color.text.subtle">brak</Text>
      ),
    },
    { key: "prefs", content: <Preferences prefs={person.preferences} emptyText="brak – obowiązują domyślne zespołu" /> },
    {
      key: "fallback",
      content: (
        <div className="fallback">
          {fallback.length > 0 && <Channels channels={fallback} />}
          {fallback.length > 0 && (
            <span aria-hidden="true"><Text color="color.text.subtle">
              →
            </Text></span>
          )}
          <Tooltip content="UI jest zawsze ostatnim kanałem zapasowym">
            <span>
              <StatusLozenge domain="channel" value="ui" />
            </span>
          </Tooltip>
        </div>
      ),
    },
  ];
}

function TeamCard({ team, people }: { team: Team; people: Map<string, Person> }) {
  return (
    <article className="team">
      <Heading size="small" as="h3">{team.name}</Heading>
      <div className="person-id">{team.id}</div>
      <Heading size="xsmall" as="h4">Członkowie ({team.members.length})</Heading>
      <div className="members">
        {team.members.map((id) => {
          const p = people.get(id);
          return (
            <Tooltip key={id} content={p ? undefined : "brak tej osoby w pliku osób"}>
              <Lozenge appearance={p ? "default" : "moved"}>{p ? `${p.name} (${id})` : `${id} (nieznana)`}</Lozenge>
            </Tooltip>
          );
        })}
      </div>
      <Heading size="xsmall" as="h4">Domyślne preferencje zespołu</Heading>
      <Preferences prefs={team.preferences} emptyText="brak" />
    </article>
  );
}

export default function People() {
  const peopleRes = useResource(() => api.GET("/people"));
  const teamsRes = useResource(() => api.GET("/teams"));

  const people = peopleRes.data?.people ?? null;
  const sourceFile = peopleRes.data?.source_file ?? null;
  const teams = teamsRes.data;
  const failure = peopleRes.error ?? teamsRes.error;
  const reload = () => {
    peopleRes.reload();
    teamsRes.reload();
  };

  const byId = new Map((people ?? []).map((p) => [p.id, p]));
  const loading = !failure && (people === null || teams === null);

  return (
    <section>
      <PageHeader>Osoby i zespoły</PageHeader>
      <ReadOnlyNote>
        Osoby, zespoły i preferencje edytuje się w pliku konfiguracyjnym; zmiany są ładowane bez restartu.
      </ReadOnlyNote>
      <Text as="p" color="color.text.subtle">
        Reguła wskazuje, KTO dostaje powiadomienie; kanały (JAK) wynikają z preferencji osoby, a gdy ich brak – z
        domyślnych preferencji zespołu. UI jest zawsze ostatnim kanałem zapasowym.
      </Text>

      {failure && <ErrorBox message={`Nie udało się pobrać danych: ${failure}`} onRetry={reload} />}
      {loading && <Loading>Ładowanie osób i zespołów…</Loading>}

      {people && teams && (
        <>
          <Card title={`Osoby (${people.length})`}>
            <SourceFile path={sourceFile} />
            {people.length === 0 ? (
              <Empty>Brak osób w pliku konfiguracyjnym.</Empty>
            ) : (
              <DynamicTable
                head={{
                  cells: [
                    { key: "person", content: "Osoba" },
                    { key: "tz", content: "Strefa czasowa" },
                    { key: "contacts", content: "Kontakty" },
                    { key: "prefs", content: "Preferencje (severity → kanały)" },
                    { key: "fallback", content: "Fallback" },
                  ],
                }}
                rows={people.map((p) => ({ key: p.id, cells: personCells(p) }))}
              />
            )}
          </Card>

          <Card title={`Zespoły (${teams.length})`}>
            <SourceFile path={sourceFile} label="Edytuj w (wg planu ten sam plik):" />
            <Text as="p" color="color.text.subtle">GET /teams nie zwraca ścieżki pliku – powyżej plik osób, w którym według planu są też zespoły.</Text>
            {teams.length === 0 ? (
              <Empty>Brak zespołów w pliku konfiguracyjnym.</Empty>
            ) : (
              <div className="team-grid">
                {teams.map((t) => (
                  <TeamCard key={t.id} team={t} people={byId} />
                ))}
              </div>
            )}
          </Card>
        </>
      )}
    </section>
  );
}
