import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installDesktopApi } from "./shared/install-desktop-api";
import "./index.css";

// Install `window.api` for the Tauri runtime before React renders. Under
// Electron the preload already exposes a frozen context-bridged `window.api`,
// so this is a no-op there.
installDesktopApi();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
