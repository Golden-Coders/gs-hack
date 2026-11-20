import { NavLink, Route, Routes } from "react-router-dom";
import AlertFeed from "./screens/AlertFeed";
import AlertDetail from "./screens/AlertDetail";
import Approvals from "./screens/Approvals";
import MockOutbox from "./screens/MockOutbox";
import Rules from "./screens/Rules";
import Mappings from "./screens/Mappings";
import Actions from "./screens/Actions";
import Runbooks from "./screens/Runbooks";
import People from "./screens/People";
import SessionBar from "./components/SessionBar";

const NAV: { to: string; label: string; group: "ops" | "config" }[] = [
  { to: "/", label: "Alerty", group: "ops" },
  { to: "/approvals", label: "Zatwierdzenia", group: "ops" },
  { to: "/mock-outbox", label: "SMS / telefony", group: "ops" },
  { to: "/rules", label: "Reguły", group: "config" },
  { to: "/mappings", label: "Mappingi", group: "config" },
  { to: "/actions", label: "Akcje", group: "config" },
  { to: "/runbooks", label: "Runbooki", group: "config" },
  { to: "/people", label: "Osoby", group: "config" },
];

export default function App() {
  return (
    <div className="layout">
      <nav>
        <div className="brand">Alerting</div>
        {(["ops", "config"] as const).map((group) => (
          <div key={group} className="nav-group">
            <div className="nav-title">{group === "ops" ? "Operacje" : "Konfiguracja"}</div>
            {NAV.filter((n) => n.group === group).map((n) => (
              <NavLink key={n.to} to={n.to} end={n.to === "/"}>
                {n.label}
              </NavLink>
            ))}
          </div>
        ))}
        <SessionBar />
      </nav>
      <main>
        <Routes>
          <Route path="/" element={<AlertFeed />} />
          <Route path="/alerts/:alertId" element={<AlertDetail />} />
          <Route path="/approvals" element={<Approvals />} />
          <Route path="/mock-outbox" element={<MockOutbox />} />
          <Route path="/rules" element={<Rules />} />
          <Route path="/mappings" element={<Mappings />} />
          <Route path="/actions" element={<Actions />} />
          <Route path="/runbooks" element={<Runbooks />} />
          <Route path="/people" element={<People />} />
        </Routes>
      </main>
    </div>
  );
}
