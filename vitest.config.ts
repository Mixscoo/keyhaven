/// <reference types="vitest" />
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dedicated config for the frontend component/store test suite (Task 14).
// Runs Vitest + @testing-library/svelte against a jsdom environment. The Tauri
// runtime is unavailable here, so `@tauri-apps/*` modules are mocked in the
// setup file (see src/test/setup.ts). The `browser` resolve condition makes
// Svelte 5 use its client (DOM) build rather than the SSR build so components
// actually mount and react to events under test.
export default defineConfig({
  plugins: [svelte()],
  resolve: {
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{js,ts}"],
  },
});
