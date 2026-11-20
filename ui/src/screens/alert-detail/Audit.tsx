import type { Schemas } from "../../api/client";
import Section from "./Section";
import { actorLabel, formatTime, type Resource } from "./util";

export default function Audit({ resource }: { resource: Resource<Schemas["AuditEntry"][]> }) {
  const entries = resource.data ?? [];
  return (
    <Section title="Dziennik działań" resource={resource}>
      {!resource.loading && !resource.error && entries.length === 0 && <p className="muted">Brak wpisów.</p>}
      {entries.length > 0 && (
        <div className="ad-table-wrap">
          <table className="ad-table">
            <thead>
              <tr>
                <th>Czas</th>
                <th>Epizod</th>
                <th>Kto</th>
                <th>Akcja</th>
                <th>Szczegóły</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr key={e.id}>
                  <td className="nowrap">{formatTime(e.at)}</td>
                  <td>#{e.episode}</td>
                  <td>{actorLabel(e.actor)}</td>
                  <td>
                    <code>{e.action}</code>
                    {e.step_id && <span className="muted"> · krok {e.step_id}</span>}
                  </td>
                  <td>{e.detail ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}
