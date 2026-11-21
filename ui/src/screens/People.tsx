// Osoby, zespoły i preferencje powiadomień (tylko do odczytu) – tasks.md 15.4.
// Reguła wskazuje KTO, a JAK (kanały) wynika z preferencji osoby; zespół daje domyślne preferencje.
import { useEffect, useState } from "react";
import { api, type Schemas } from "../api/client";
import { SeverityBadge } from "../components/Badges";
import SourceFile, { ReadOnlyNote } from "../components/SourceFile";
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
  const [people, setPeople] = useState<Person[] | null>(null);
  const [sourceFile, setSourceFile] = useState<string | null>(null);
  const [teams, setTeams] = useState<Team[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [p, t] = await Promise.all([api.GET("/people"), api.GET("/teams")]);
        if (cancelled) return;
        if (p.error || !p.data) throw new Error("GET /people nie powiodło się");
        if (t.error || !t.data) throw new Error("GET /teams nie powiodło się");
        setPeople(p.data.people);
        setSourceFile(p.data.source_file);
        setTeams(t.data);
      } catch (e) {
        if (!cancelled) setFailure(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

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

      {failure && (
        <div className="error-box" role="alert">
          Nie udało się pobrać danych: {failure}
        </div>
      )}
      {loading && <p className="loading">Ładowanie osób i zespołów…</p>}

      {people && teams && (
        <>
          <div className="people-section">
            <h2>Osoby ({people.length})</h2>
            <SourceFile path={sourceFile} />
            {people.length === 0 ? (
              <p className="empty">Brak osób w pliku konfiguracyjnym.</p>
            ) : (
              <div className="table-wrap">
                <table className="people-table">
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
          </div>

          <div className="people-section">
            <h2>Zespoły ({teams.length})</h2>
            <SourceFile path={sourceFile} label="Edytuj w (wg planu ten sam plik):" />
            <p className="muted">GET /teams nie zwraca ścieżki pliku – powyżej plik osób, w którym według planu są też zespoły.</p>
            {teams.length === 0 ? (
              <p className="empty">Brak zespołów w pliku konfiguracyjnym.</p>
            ) : (
              <div className="team-grid">
                {teams.map((t) => (
                  <TeamCard key={t.id} team={t} people={byId} />
                ))}
              </div>
            )}
          </div>
        </>
      )}
    </section>
  );
}
