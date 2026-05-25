import type { ElectronAPI } from "../electron/preload";

declare global {
  interface Window {
    api: ElectronAPI;
  }
}
