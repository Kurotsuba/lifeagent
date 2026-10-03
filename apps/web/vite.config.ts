import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// LifeAgent UI: thin view layer, no business logic, served by `lifeagent-server`
// (`src/api/transport.ts`). See docs/PHASE1_SPEC.md §10.3.
export default defineConfig({
  plugins: [react(), tailwindcss()],

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Development: open http://localhost:1420 in a browser with `cargo run -p lifeagent-server`
    // running.
    proxy: {
      "/api": "http://localhost:8080",
    },
  },
  build: {
    // `safari13` is left out: with it, this toolchain's esbuild pass fails on ordinary array
    // destructuring ("Transforming destructuring to the configured target environment... is not
    // supported yet").
    target: ["es2021", "chrome100"],
  },
});
