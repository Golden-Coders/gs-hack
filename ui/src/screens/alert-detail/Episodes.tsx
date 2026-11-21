import type { Schemas } from "../../api/client";
import Section from "./Section";
import type { Resource } from "../../api/useResource";
import { actorLabel, formatTime } from "../../ui/format";

export default function Episodes({ resource }: { resource: Resource<Schemas["Episode"][]> }) {
  const episodes = resource.data ?? [];
  return (
    <Section title="Epizody" resource={resource}>
      {!resource.loading && !resource.error && episodes.length === 0 && <p className="muted">Brak epizodów.</p>}
      {episodes.length > 0 && (
        <div className="ad-table-wrap">
          <table className="ad-table">
            <thead>
              <tr>
                <th>Epizod</th>
                <th>Otwarty</th>
                <th>Potwierdzony</th>
                <th>Rozwiązany</th>
                <th className="num">Licznik</th>
                <th>Rozwiązał</th>
              </tr>
            </thead>
            <tbody>
              {episodes.map((e) => (
                <tr key={e.episode}>
                  <td>#{e.episode}</td>
                  <td className="nowrap">{formatTime(e.opened_at)}</td>
                  <td className="nowrap">{formatTime(e.acked_at)}</td>
                  <td className="nowrap">{formatTime(e.resolved_at)}</td>
                  <td className="num">{e.count}</td>
                  <td>{actorLabel(e.resolved_by)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}
