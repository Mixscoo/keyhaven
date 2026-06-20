import { get, writable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { localDataDir, join } from "@tauri-apps/api/path";
import type { SessionStatus, VaultSummary } from "../types";
import * as api from "../api";
import { resetEntries } from "./entries";
import { searchQuery } from "./searchQuery";
import { loadSettings, resetSettings } from "./settings";
import { resetNavigation } from "./navigation";
import { toast } from "./toast";

export interface SessionState {
  status: SessionStatus;
  summary?: VaultSummary;
}

// Session status drives top-level routing (no-vault → Setup, locked → Unlock,
// unlocked → Vault). The backend is the source of truth; this store mirrors it.
export const session = writable<SessionState>({ status: "no-vault" });

// Filesystem path of the active vault. Resolved to a clean per-user app folder
// on startup; the import/open flows (later tasks) may repoint it.
export const vaultPath = writable<string>("");

const DEFAULT_VAULT_FILENAME = "keyhaven.khv";
// Plain, human-readable folder name under the OS local data dir (e.g. on Windows
// `%LOCALAPPDATA%\Keyhaven`), instead of the dotted bundle identifier. Local
// (not Roaming) keeps the encrypted vault pinned to this machine and never
// syncs it to a domain roaming server, matching Keyhaven's offline-first design.
const APP_FOLDER_NAME = "Keyhaven";

/** The default vault location: `<local data dir>/Keyhaven/keyhaven.khv`. */
export async function resolveDefaultVaultPath(): Promise<string> {
  const dir = await localDataDir();
  return join(dir, APP_FOLDER_NAME, DEFAULT_VAULT_FILENAME);
}

/**
 * Determine the initial session status from the backend and set the route.
 *
 * The backend is authoritative about the unlocked state; if it is not unlocked
 * we route to Unlock when a vault file exists, or to Setup when none does.
 */
export async function initSession(): Promise<void> {
  let path = get(vaultPath);
  if (!path) {
    path = await resolveDefaultVaultPath();
    vaultPath.set(path);
  }

  if (await api.isUnlocked()) {
    session.set({ status: "unlocked" });
    await loadSettings();
    return;
  }

  const exists = await api.vaultExists(path);
  session.set({ status: exists ? "locked" : "no-vault" });
}

/**
 * Enter the unlocked shell after a successful unlock or create. Loads settings
 * for the now-open vault. Call with the {@link VaultSummary} the backend
 * returned from the unlock/create command.
 */
export async function enterUnlocked(summary: VaultSummary): Promise<void> {
  session.set({ status: "unlocked", summary });
  resetNavigation();
  try {
    await loadSettings();
  } catch {
    // Non-fatal: the unlocked shell can still render without settings loaded.
    toast.push("warning", "Could not load settings.");
  }
}

/**
 * Clear all in-memory decrypted state and route back to the Unlock screen.
 * Shared by manual lock, the auto-lock timeout, and lock-on-blur. A vault file
 * must exist for us to have been unlocked, so the post-lock route is Unlock.
 */
function clearToLocked(): void {
  resetEntries();
  resetSettings();
  resetNavigation();
  searchQuery.set("");
  session.set({ status: "locked" });
}

/**
 * Manually lock the vault: ask the backend to lock (it zeroizes the VEK and
 * decrypted model and emits `vault-locked`), then clear local state eagerly so
 * the UI responds immediately. The `vault-locked` listener will also fire; the
 * clearing is idempotent.
 */
export async function lock(): Promise<void> {
  await api.lockVault();
  clearToLocked();
}

/**
 * Subscribe to the backend `vault-locked` event (emitted on auto-lock timeout,
 * lock-on-blur, or an explicit lock). On lock we drop all decrypted in-memory
 * state and route to Unlock. Returns an unlisten function for teardown.
 */
export function listenForLock(): Promise<UnlistenFn> {
  return listen("vault-locked", () => {
    clearToLocked();
  });
}
