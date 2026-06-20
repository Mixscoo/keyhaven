import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri 2 + Svelte (SPA mode) frontend.
// Single-page app: a single index.html entry, client-side view switching only.
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],

  // Prevent Vite from obscuring Rust errors.
  clearScreen: false,

  // Tauri expects a fixed port and fails if it is not available.
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Tell Vite to ignore watching `src-tauri`.
      ignored: ["**/src-tauri/**"],
    },
  },

  // Produce a lean SPA build.
  build: {
    target: "esnext",
    sourcemap: false,
  },
});
