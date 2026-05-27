import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import electron from "vite-plugin-electron";

export default defineConfig({
  test: {
    testTimeout: 15000,
  },
  plugins: [
    react(),
    electron([
      {
        entry: "electron/main.ts",
        vite: {
          build: {
            outDir: "dist-electron",
            rollupOptions: {
              external: [
                "better-sqlite3",
                "music-metadata",
                "node-id3",
                "opencc-js",
                "sharp",
              ],
            },
          },
        },
      },
      {
        entry: "electron/preload.ts",
        onstart(args) {
          args.reload();
        },
        vite: {
          build: {
            outDir: "dist-electron",
            rollupOptions: {
              output: {
                entryFileNames: "preload.mjs",
                format: "es",
              },
            },
          },
        },
      },
    ]),
  ],
  build: {
    outDir: "dist",
  },
});
