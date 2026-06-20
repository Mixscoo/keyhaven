import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, vi } from "vitest";
import { cleanup } from "@testing-library/svelte";
import { resetTauriMocks } from "./tauri-mocks";

/*
 * Global test setup (Task 14).
 *
 * The frontend talks to a trusted Rust backend exclusively through Tauri's
 * `invoke` and a few plugin APIs. None of that exists under jsdom, so we mock
 * every `@tauri-apps/*` module the app touches and delegate to the shared
 * spies in `./tauri-mocks`. Registering the mocks here means they apply to
 * every test file without per-file boilerplate.
 */

vi.mock("@tauri-apps/api/core", async () => {
  const { invoke } = await import("./tauri-mocks");
  return { invoke: (...args: unknown[]) => invoke(...args) };
});

vi.mock("@tauri-apps/plugin-clipboard-manager", async () => {
  const { writeText } = await import("./tauri-mocks");
  return { writeText: (...args: unknown[]) => writeText(...args) };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listen } = await import("./tauri-mocks");
  return { listen: (...args: unknown[]) => listen(...args) };
});

vi.mock("@tauri-apps/api/path", async () => {
  const { appDataDir, dataDir, localDataDir, join } = await import("./tauri-mocks");
  return { appDataDir, dataDir, localDataDir, join };
});

beforeEach(() => {
  resetTauriMocks();
});

afterEach(() => {
  cleanup();
});
