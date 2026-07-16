import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installDesktopApi } from "./shared/install-desktop-api";
import "./index.css";

// Install the Tauri-backed `window.api` contract before React renders.
installDesktopApi();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
