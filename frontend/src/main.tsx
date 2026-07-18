import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installDesktopApi } from "./shared/install-desktop-api";
import "./index.css";

async function bootstrap() {
  // The WebdriverIO bridge and embedded driver are compiled only for the
  // dedicated E2E build. Production bundles never import the test bridge.
  if (import.meta.env.MODE === "wdio") {
    await import("@wdio/tauri-plugin");
  }

  // Install the Tauri-backed `window.api` contract before React renders.
  installDesktopApi();

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}

void bootstrap();
