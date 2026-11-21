// Osoby, zespoły i preferencje powiadomień (tylko do odczytu) – tasks.md 15.4.
// Reguła wskazuje KTO, a JAK (kanały) wynika z preferencji osoby; zespół daje domyślne preferencje.
import { api, type Schemas } from "../api/client";
import { useResource } from "../api/useResource";
import { SeverityBadge } from "../components/Badges";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
import { Card, Empty, ErrorBox, Loading } from "../ui/primitives";
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
          {i > 0 && <span className="pref-arrow" aria-hidden="true">{" → "}</span>}
          <span className="channel-chip">{c}</span>
        </span>
      ))}
    </>
  );
}

function Preferences({ prefs, emptyText }: { prefs?: Preference[]; emptyText: string }) {
  if (!prefs || prefs.length === 0) return <span className="muted">{emptyText}</span>;
  const sorted = [...prefs].sort((a, b) => SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity]);
  return (
    <ul className="prefs">
      {sorted.map((p, i) => (
        <li key={i} className="pref">
          <SeverityBadge severity={p.severity} />
          <span className="pref-arrow" aria-hidden="true">
            →
          </span>
          <Channels channels={p.channels} />
          <span className="pref-hours">{p.hours ? `godz. ${p.hours}` : "całą dobę"}</span>
        </li>
      ))}
    </ul>
  );
}

function PersonRow({ person }: { person: Person }) {
  const contacts = Object.entries(person.contacts ?? {});
  const fallback = person.fallback ?? [];
  return (
    <tr>
      <td>
        <div className="person-name">{person.name}</div>
        <div className="person-id">{person.id}</div>
      </td>
      <td>{person.timezone ?? <span className="muted">—</span>}</td>
      <td>
        {contacts.length ? (
          <ul className="contacts">
            {contacts.map(([channel, value]) => (
              <li key={channel}>
                <span className="contact-channel">{channel}</span>
                <span className="mono">{value}</span>
              </li>
            ))}
          </ul>
        ) : (
          <span className="muted">brak</span>
        )}
      </td>
      <td>
        <Preferences prefs={person.preferences} emptyText="brak – obowiązują domyślne zespołu" />
      </td>
      <td>
        <div className="fallback">
          {fallback.length > 0 && <Channels channels={fallback} />}
          {fallback.length > 0 && (
            <span className="pref-arrow" aria-hidden="true">
              →
            </span>
          )}
          <span className="channel-chip always" title="UI jest zawsze ostatnim kanałem zapasowym">
            ui
          </span>
        </div>
      </td>
    </tr>
  );
}

function TeamCard({ team, people }: { team: Team; people: Map<string, Person> }) {
  return (
    <article className="team">
      <h3>{team.name}</h3>
      <div className="person-id">{team.id}</div>
      <h4>Członkowie ({team.members.length})</h4>
      <div className="members">
        {team.members.map((id) => {
          const p = people.get(id);
          return (
            <span key={id} className="rule-chip" title={p ? undefined : "brak tej osoby w pliku osób"}>
              {p ? `${p.name} (${id})` : `${id} (nieznana)`}
            </span>
          );
        })}
      </div>
      <h4>Domyślne preferencje zespołu</h4>
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
      <h1>Osoby i zespoły</h1>
      <ReadOnlyNote>
        Osoby, zespoły i preferencje edytuje się w pliku konfiguracyjnym; zmiany są ładowane bez restartu.
      </ReadOnlyNote>
      <p className="ui-fallback-note">
        Reguła wskazuje, KTO dostaje powiadomienie; kanały (JAK) wynikają z preferencji osoby, a gdy ich brak – z
        domyślnych preferencji zespołu. UI jest zawsze ostatnim kanałem zapasowym.
      </p>

      {failure && <ErrorBox message={`Nie udało się pobrać danych: ${failure}`} onRetry={reload} />}
      {loading && <Loading>Ładowanie osób i zespołów…</Loading>}

      {people && teams && (
        <>
          <Card title={`Osoby (${people.length})`}>
            <SourceFile path={sourceFile} />
            {people.length === 0 ? (
              <Empty>Brak osób w pliku konfiguracyjnym.</Empty>
            ) : (
              <div className="table-wrap">
                <table className="data-table people-table">
                  <thead>
                    <tr>
                      <th scope="col">Osoba</th>
                      <th scope="col">Strefa czasowa</th>
                      <th scope="col">Kontakty</th>
                      <th scope="col">Preferencje (severity → kanały)</th>
                      <th scope="col">Fallback</th>
                    </tr>
                  </thead>
                  <tbody>
                    {people.map((p) => (
                      <PersonRow key={p.id} person={p} />
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </Card>

          <Card title={`Zespoły (${teams.length})`}>
            <SourceFile path={sourceFile} label="Edytuj w (wg planu ten sam plik):" />
            <p className="muted">GET /teams nie zwraca ścieżki pliku – powyżej plik osób, w którym według planu są też zespoły.</p>
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
