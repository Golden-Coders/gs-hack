import "@atlaskit/css-reset";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { FlagsProvider } from "@atlaskit/flag";
import { setGlobalTheme } from "@atlaskit/tokens/set-global-theme";
import App from "./App";
import "./styles.css";
import "./ui/ui.css";

setGlobalTheme({ colorMode: "auto", light: "light", dark: "dark", spacing: "spacing", typography: "typography" }).finally(
  () =>
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <FlagsProvider>
          <BrowserRouter>
            <App />
          </BrowserRouter>
        </FlagsProvider>
      </StrictMode>,
    ),
);
