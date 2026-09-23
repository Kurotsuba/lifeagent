import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// LifeAgent desktop UI: thin view layer over Tauri commands, no business logic.
// See docs/PHASE1_SPEC.md §10.3.
export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Tauri expects a fixed dev-server port and will fail if it is unavailable.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: ["es2021", "chrome100", "safari13"],
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
