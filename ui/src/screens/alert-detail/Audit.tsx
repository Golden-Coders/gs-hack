import type { Schemas } from "../../api/client";
import { Code } from "@atlaskit/code";
import { Card, Empty } from "../../ui/primitives";
import type { Resource } from "../../api/useResource";
import { actorLabel, formatTime } from "../../ui/format";

export default function Audit({ resource }: { resource: Resource<Schemas["AuditEntry"][]> }) {
  const entries = resource.data ?? [];
  return (
    <Card title="Dziennik działań" loading={resource.loading} error={resource.error} onRetry={resource.reload}>
      {!resource.loading && !resource.error && entries.length === 0 && <Empty>Brak wpisów.</Empty>}
      {entries.length > 0 && (
        <div className="table-wrap">
          <table className="data-table">
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
                    <Code>{e.action}</Code>
                    {e.step_id && <span className="muted"> · krok {e.step_id}</span>}
                  </td>
                  <td>{e.detail ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}
