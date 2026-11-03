/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed port and does not want the screen cleared.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  // Window tests: a fake browser (jsdom) and a fake backend. See src/test/.
  test: { environment: "jsdom", setupFiles: ["src/test/setup.ts"] },
});
