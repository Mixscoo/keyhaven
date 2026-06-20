/*
 * Thin, typed wrappers over Tauri `invoke` commands. The frontend NEVER performs
 * cryptography or networking; it only calls into the trusted Rust backend.
 *
 * Each wrapper maps 1:1 to a handler in the Rust command surface
 * (`src-tauri/src/commands.rs`, registered in `src-tauri/src/lib.rs`). Argument
 * names and result shapes match the backend's serde contract exactly — see
 * `./types` for the field-naming rationale (camelCase command structs vs.
 * snake_case model types).
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  CatalogService,
  CreateVaultResult,
  CustomService,
  Entry,
  EntryInput,
  EntryList,
  EntrySaveResult,
  IconRef,
  Page,
  PasswordGenOptions,
  Settings,
  VaultSummary,
} from "./types";

// ---- Vault lifecycle ----

/** Whether a vault file already exists at `path` (used for startup routing). */
export const vaultExists = (path?: string): Promise<boolean> =>
  invoke("vault_exists", { path });

/**
 * Create a new encrypted vault at `path`. Returns the recovery key exactly once
 * when `generateRecovery` is true. Leaves the vault unlocked on success.
 */
export const createVault = (
  masterPassword: string,
  generateRecovery: boolean,
  path: string,
): Promise<CreateVaultResult> =>
  invoke("create_vault", { masterPassword, generateRecovery, path });

export const unlockWithPassword = (
  masterPassword: string,
  path: string,
): Promise<VaultSummary> =>
  invoke("unlock_with_password", { masterPassword, path });

export const unlockWithRecoveryKey = (
  recoveryKey: string,
  path: string,
): Promise<VaultSummary> =>
  invoke("unlock_with_recovery_key", { recoveryKey, path });

export const changeMasterPassword = (
  current: string,
  newPassword: string,
): Promise<void> => invoke("change_master_password", { current, newPassword });

export const lockVault = (): Promise<void> => invoke("lock_vault");

export const isUnlocked = (): Promise<boolean> => invoke("is_unlocked");

/**
 * Report qualifying user activity so the backend resets its auto-lock
 * inactivity countdown. The authoritative timer lives in the backend; this only
 * nudges the deadline forward and is a no-op while locked.
 */
export const reportActivity = (): Promise<void> => invoke("report_activity");

// ---- Entries (require an unlocked session) ----

export const listEntries = (query?: string, page?: Page): Promise<EntryList> =>
  invoke("list_entries", { query, page });

export const getEntry = (id: string): Promise<Entry> => invoke("get_entry", { id });

export const createEntry = (input: EntryInput): Promise<EntrySaveResult> =>
  invoke("create_entry", { input });

export const updateEntry = (
  id: string,
  input: EntryInput,
): Promise<EntrySaveResult> => invoke("update_entry", { id, input });

export const deleteEntry = (id: string): Promise<void> =>
  invoke("delete_entry", { id });

// ---- Services / catalog ----

export const searchCatalog = (query: string): Promise<CatalogService[]> =>
  invoke("search_catalog", { query });

export const listCustomServices = (): Promise<CustomService[]> =>
  invoke("list_custom_services");

export const createCustomService = (
  name: string,
  icon: IconRef,
): Promise<CustomService> => invoke("create_custom_service", { name, icon });

// ---- Utilities ----

export const generatePassword = (opts: PasswordGenOptions): Promise<string> =>
  invoke("generate_password", { opts });

export const copySecretToClipboard = (
  entryId: string,
  fieldId: string,
): Promise<void> => invoke("copy_secret_to_clipboard", { entryId, fieldId });

export const exportVault = (destination: string): Promise<void> =>
  invoke("export_vault", { destination });

export const importVault = (source: string): Promise<void> =>
  invoke("import_vault", { source });

/**
 * Install an external vault file as this device's vault (first-run "I already
 * have a vault" flow): validates `source` and copies it to `destination` (the
 * device's default vault path). Refuses to overwrite an existing vault. The
 * caller then routes to Unlock to enter the vault's original master password.
 */
export const importExternalVault = (
  source: string,
  destination: string,
): Promise<void> => invoke("import_external_vault", { source, destination });

/**
 * Save the one-time recovery key to a plaintext file the user chose via the
 * native save dialog. The key is written by the trusted backend; the frontend
 * only supplies the destination path and the key it just displayed.
 */
export const saveRecoveryKey = (
  destination: string,
  recoveryKey: string,
): Promise<void> => invoke("save_recovery_key", { destination, recoveryKey });

// ---- Settings ----

export const getSettings = (): Promise<Settings> => invoke("get_settings");

export const updateSettings = (settings: Settings): Promise<void> =>
  invoke("update_settings", { settings });
