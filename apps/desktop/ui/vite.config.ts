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
    // `safari13` (the M0 scaffold's original, generic-Tauri-template value, alongside
    // `chrome100` for Windows WebView2/Linux WebKitGTK) is dropped: CLAUDE.md scopes this
    // project to Windows only, and keeping it hits a real bug in this toolchain's minifier —
    // `vite build`'s esbuild pass fails on ordinary array destructuring (e.g. `const [a, b] =
    // useState(...)`, introduced by M7.2's onboarding UI) with "Transforming destructuring to
    // the configured target environment... is not supported yet" specifically when `safari13`
    // is in the target list. Confirmed by reproducing with `minify: false`, which built fine.
    target: ["es2021", "chrome100"],
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
