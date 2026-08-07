import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // Tauri serves the built files; no network access at runtime.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    // Debug symbols only in dev builds.
    sourcemap: process.env.TAURI_DEBUG ? "inline" : false,
  },
});
