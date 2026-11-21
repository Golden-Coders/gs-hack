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
import { lazy, Suspense } from "react";
import { Box } from "@atlaskit/primitives/compiled";
import { matchPath, NavLink, Route, Routes, useLocation } from "react-router-dom";
import SessionBar from "./components/SessionBar";
import { Loading } from "./ui/primitives";
import AlertFeed from "./screens/AlertFeed";

// D14: ekrany poza startowym feedem w osobnych chunkach (główny chunk > 500 kB), patrz bundle.md.
const AlertDetail = lazy(() => import("./screens/AlertDetail"));
const Approvals = lazy(() => import("./screens/Approvals"));
const MockOutbox = lazy(() => import("./screens/MockOutbox"));
const Rules = lazy(() => import("./screens/Rules"));
const Mappings = lazy(() => import("./screens/Mappings"));
const Actions = lazy(() => import("./screens/Actions"));
const Runbooks = lazy(() => import("./screens/Runbooks"));
const People = lazy(() => import("./screens/People"));
const DevComponents = lazy(() => import("./screens/DevComponents"));

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
            <Suspense fallback={<Loading />}>
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
            </Suspense>
          </Box>
        </Main>
      </Content>
    </PageLayout>
  );
}
