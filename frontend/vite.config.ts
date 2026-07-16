import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  test: {
    testTimeout: 15000,
    exclude: ["node_modules/**", "dist/**", "e2e/**"],
  },
  plugins: [react()],
  build: {
    outDir: "dist",
  },
});
