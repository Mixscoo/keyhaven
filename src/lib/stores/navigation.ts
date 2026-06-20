import { writable } from "svelte/store";

/**
 * In-shell navigation for the unlocked vault. Top-level session routing
 * (no-vault → Setup, locked → Unlock, unlocked → shell) lives in `session.ts`
 * and `App.svelte`; this store only switches between screens *within* the
 * unlocked shell (the entry list vs. the entry editor).
 *
 * Kept deliberately small: a tagged union so the editor can carry the id of the
 * entry being edited (absent `entryId` means "create new").
 */
export type ShellRoute =
  | { name: "list" }
  | { name: "editor"; entryId?: string }
  | { name: "settings" };

export const route = writable<ShellRoute>({ name: "list" });

/** Open the entry editor. Omit `entryId` to create a new entry. */
export function openEditor(entryId?: string): void {
  route.set({ name: "editor", entryId });
}

/** Open the settings screen. */
export function openSettings(): void {
  route.set({ name: "settings" });
}

/** Return to the main entry list. */
export function openList(): void {
  route.set({ name: "list" });
}

/** Reset to the default screen. Used on lock/unlock so no stale route lingers. */
export function resetNavigation(): void {
  route.set({ name: "list" });
}
