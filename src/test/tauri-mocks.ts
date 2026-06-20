import { vi } from "vitest";

/*
 * Shared mock functions standing in for the Tauri runtime, which is not present
 * when tests run under Vitest/jsdom. The actual `vi.mock(...)` registrations
 * live in `setup.ts` and delegate to these functions so individual tests can
 * import them here and tailor behavior per case (e.g. route `invoke` by command
 * name, capture clipboard writes, drive the `vault-locked` event).
 */

/** Stand-in for `@tauri-apps/api/core`'s `invoke`. */
export const invoke = vi.fn();

/** Stand-in for `@tauri-apps/plugin-clipboard-manager`'s `writeText`. */
export const writeText = vi.fn();

/** Stand-in for `@tauri-apps/api/event`'s `listen`. */
export const listen = vi.fn();

/** Stand-in for `@tauri-apps/api/path` helpers. */
export const appDataDir = vi.fn(async () => "/app-data");
export const dataDir = vi.fn(async () => "/data");
export const localDataDir = vi.fn(async () => "/local-data");
export const join = vi.fn(async (...parts: string[]) => parts.join("/"));

/** Reset every mock to a clean slate between tests. */
export function resetTauriMocks(): void {
  invoke.mockReset();
  writeText.mockReset();
  listen.mockReset();
  appDataDir.mockReset();
  appDataDir.mockImplementation(async () => "/app-data");
  dataDir.mockReset();
  dataDir.mockImplementation(async () => "/data");
  localDataDir.mockReset();
  localDataDir.mockImplementation(async () => "/local-data");
  join.mockReset();
  join.mockImplementation(async (...parts: string[]) => parts.join("/"));
  // Default: clipboard writes succeed unless a test overrides this.
  writeText.mockResolvedValue(undefined);
  // Default: event subscriptions return a no-op unlisten function.
  listen.mockResolvedValue(() => {});
}
