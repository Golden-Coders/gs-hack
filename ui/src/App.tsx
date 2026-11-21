// Decyzja człowieka (design.md D1): powłoka Nav 3 (page-layout + side-navigation), nie navigation-system.
// eslint-disable-next-line @atlaskit/design-system/no-deprecated-imports -- j.w.
import { Content, LeftSidebar, Main, PageLayout } from "@atlaskit/page-layout";
// eslint-disable-next-line @atlaskit/design-system/no-deprecated-imports -- j.w.
import {
  CustomItem,
  Header,
  NavigationContent,
  NavigationFooter,
  NavigationHeader,
  Section,
  SideNavigation,
  type CustomItemComponentProps,
} from "@atlaskit/side-navigation";
import { Box } from "@atlaskit/primitives/compiled";
import { matchPath, NavLink, Route, Routes, useLocation } from "react-router-dom";
import AlertFeed from "./screens/AlertFeed";
import AlertDetail from "./screens/AlertDetail";
import Approvals from "./screens/Approvals";
import MockOutbox from "./screens/MockOutbox";
import Rules from "./screens/Rules";
import Mappings from "./screens/Mappings";
import Actions from "./screens/Actions";
import Runbooks from "./screens/Runbooks";
import People from "./screens/People";
import DevComponents from "./screens/DevComponents";
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

// Pozycja menu jako link routera (CustomItem `component`); NavLink sam ustawia aria-current="page".
function NavLinkItem({ children, to, ...props }: CustomItemComponentProps & { to: string }) {
  return (
    <NavLink to={to} end={to === "/"} {...props}>
      {children}
    </NavLink>
  );
}

export default function App() {
  const { pathname } = useLocation();
  return (
    <PageLayout>
      <Content>
        {/* LeftSidebar (z przyciskiem zwijania): poniżej 768 px page-layout zwija go do wysuwanego panelu. */}
        <LeftSidebar
          id="nawigacja"
          skipLinkTitle="Nawigacja"
          width={240}
          resizeButtonLabel="Nawigacja"
          resizeGrabAreaLabel="Zmień szerokość nawigacji"
          valueTextLabel="Szerokość"
        >
          <SideNavigation label="Nawigacja">
            <NavigationHeader>
              <Header>Alerting</Header>
            </NavigationHeader>
            <NavigationContent>
              {(["ops", "config"] as const).map((group) => (
                <Section key={group} title={group === "ops" ? "Operacje" : "Konfiguracja"} hasSeparator={group === "config"}>
                  {NAV.filter((n) => n.group === group).map((n) => (
                    <CustomItem
                      key={n.to}
                      component={NavLinkItem}
                      to={n.to}
                      isSelected={!!matchPath({ path: n.to, end: n.to === "/" }, pathname)}
                    >
                      {n.label}
                    </CustomItem>
                  ))}
                </Section>
              ))}
            </NavigationContent>
            <NavigationFooter>
              <SessionBar />
            </NavigationFooter>
          </SideNavigation>
        </LeftSidebar>
        <Main id="tresc" skipLinkTitle="Treść">
          <Box padding="space.300">
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
              <Route path="/dev/components" element={<DevComponents />} />
            </Routes>
          </Box>
        </Main>
      </Content>
    </PageLayout>
  );
}
