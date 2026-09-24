import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port in development and serves the built files from
// dist/ in release.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // xterm.js is most of the bundle; the app is loaded from disk, not a network.
  build: { target: "es2022", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1500 },
  test: { include: ["src/**/*.test.ts"] },
});
