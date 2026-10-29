import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installDesktopApi, isWebRuntime } from "./shared/install-desktop-api";
import { WebApp } from "./web/WebApp";
import "./index.css";

async function bootstrap() {
  // The WebdriverIO bridge and embedded driver are compiled only for the
  // dedicated E2E build. Production bundles never import the test bridge.
  if (import.meta.env.MODE === "wdio") {
    await import("@wdio/tauri-plugin");
  }

  // Install the runtime `window.api` contract before React renders. This also
  // records which runtime is active.
  installDesktopApi();

  // The browser runtime mounts its own application. The web UI is independent
  // of the standalone desktop composition in `App.tsx`; the two share the Rust
  // backend and the `DesktopAPI` contract, not layout or navigation.
  const Root = isWebRuntime() ? WebApp : App;

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <Root />
    </React.StrictMode>
  );
}

void bootstrap();
