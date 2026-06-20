import { writable } from "svelte/store";
import type { Settings } from "../types";
import * as api from "../api";

// Defaults match the backend's Settings::default() (5-minute auto-lock,
// lock-on-blur off, 20-second clipboard clear, 20-char generated passwords).
const defaults: Settings = {
  auto_lock_seconds: 300,
  lock_on_blur: false,
  clipboard_clear_seconds: 20,
  password_gen_defaults: {
    length: 20,
    upper: true,
    lower: true,
    digits: true,
    symbols: true,
  },
};

// Mirror of backend settings (which are stored encrypted within the vault).
export const settings = writable<Settings>(defaults);

/** Load settings from the unlocked vault into the store. */
export async function loadSettings(): Promise<void> {
  const loaded = await api.getSettings();
  settings.set(loaded);
}

/** Persist `next` to the vault and update the store on success. */
export async function saveSettings(next: Settings): Promise<void> {
  await api.updateSettings(next);
  settings.set(next);
}

/** Reset to defaults. Used on lock so stale settings don't leak across sessions. */
export function resetSettings(): void {
  settings.set(defaults);
}
