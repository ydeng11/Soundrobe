import type { DesktopAPI } from "./shared/desktop-api";

declare global {
  interface Window {
    api: DesktopAPI;
  }
}