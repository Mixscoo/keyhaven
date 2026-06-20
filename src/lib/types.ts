/*
 * Shared frontend types mirroring the Rust backend (model.rs, commands.rs,
 * entries.rs, catalog, generator, error). These shapes match the EXACT JSON the
 * backend emits/accepts over Tauri IPC — there is no automatic case conversion,
 * so the field naming here deliberately follows the backend serde settings:
 *
 *  - Command result/argument types use camelCase (the command structs derive
 *    `#[serde(rename_all = "camelCase")]`).
 *  - Persisted model types (Entry, Settings, ...) use the model's default serde
 *    naming, which is snake_case.
 *
 * Where the two differ for the same concept (e.g. `Entry.service_ref` vs.
 * `EntrySummary.serviceRef`) the difference is intentional and documented.
 */

export type SessionStatus = "no-vault" | "locked" | "unlocked";

// ---------------------------------------------------------------------------
// Command result types (camelCase)
// ---------------------------------------------------------------------------

/** Non-secret summary returned by the unlock commands. */
export interface VaultSummary {
  hasRecovery: boolean;
  entryCount: number;
  /** True when unlocked via the recovery key (UI then prompts for a new password). */
  unlockedViaRecovery: boolean;
}

/** Result of `create_vault`; `recoveryKey` is present only when one was generated. */
export interface CreateVaultResult {
  recoveryKey?: string;
}

// ---------------------------------------------------------------------------
// Model types (snake_case — the decrypted vault model)
// ---------------------------------------------------------------------------

export type FieldType =
  | "email"
  | "username"
  | "password"
  | "phone"
  | "url"
  | "text"
  | "note"
  | "totp_secret"
  | "recovery_code";

/** Reference to the service an entry belongs to (tagged union on `kind`). */
export type ServiceRef =
  | { kind: "catalog"; id: string }
  | { kind: "custom"; id: string };

/** Reference to a service icon (tagged union on `kind`). */
export type IconRef =
  | { kind: "builtin"; ref: string }
  | { kind: "data"; ref: string };

export interface EntryField {
  id: string;
  label: string;
  /** Serialized as `type` on the wire. */
  type: FieldType;
  value: string;
  secret: boolean;
}

/** A full entry as returned by `get_entry` (model serialization → snake_case). */
export interface Entry {
  id: string;
  service_ref: ServiceRef;
  title?: string;
  fields: EntryField[];
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// Entry create/update input (matches the backend EntryInput deserialize shape)
// ---------------------------------------------------------------------------

export interface FieldInput {
  /** Existing field id to preserve, or omit for a newly added field. */
  id?: string;
  label: string;
  /** Serialized as `type` on the wire. */
  type: FieldType;
  value: string;
  secret: boolean;
}

export interface EntryInput {
  service_ref: ServiceRef;
  title?: string;
  fields: FieldInput[];
}

/**
 * UI-only working shape for a field while it is being edited. Mirrors
 * `EntryField` but uses a client-generated temp id (prefixed `tmp-`) for fields
 * that don't yet exist in the vault, so the editor can track rows before a save
 * assigns real ids. Not sent over IPC directly — mapped to {@link FieldInput}
 * on save (temp ids are dropped so the backend treats them as new fields).
 */
export interface WorkingField {
  id: string;
  label: string;
  type: FieldType;
  value: string;
  secret: boolean;
}

/** Result of `create_entry` / `update_entry`. */
export interface EntrySaveResult {
  entry: Entry;
  /** True when the saved entry has no content; the UI warns but does not block. */
  emptyWarning: boolean;
}

// ---------------------------------------------------------------------------
// Listing / search (camelCase command serialization)
// ---------------------------------------------------------------------------

/** Lightweight, non-secret entry summary for the list view. */
export interface EntrySummary {
  id: string;
  serviceRef: ServiceRef;
  title?: string;
  /** A short, non-secret preview. Never contains a secret value. */
  snippet?: string;
}

/** A page of summaries plus pagination metadata. */
export interface EntryList {
  entries: EntrySummary[];
  total: number;
  offset: number;
  limit: number;
}

/** Pagination request for `list_entries`. */
export interface Page {
  offset: number;
  limit: number;
}

// ---------------------------------------------------------------------------
// Catalog & custom services
// ---------------------------------------------------------------------------

export interface RecommendedField {
  label: string;
  /** Serialized as `type` on the wire. */
  type: FieldType;
  secret: boolean;
}

/** A bundled catalog service (model serialization → snake_case). */
export interface CatalogService {
  id: string;
  name: string;
  icon: string;
  /** Inline brand-logo SVG markup (brand-colored). Empty when none. */
  svg?: string;
  /** The logo's primary hex color (e.g. "#4285f4"), for tinting the tile. */
  color?: string;
  /** A raster logo as a `data:` URL (bundled favicon). Empty when none. */
  icon_data?: string;
  aliases: string[];
  recommended_fields: RecommendedField[];
}

/** A user-defined custom service as returned by the command layer (camelCase). */
export interface CustomService {
  id: string;
  name: string;
  icon: IconRef;
  /** Always true — marks this as a user-defined custom service. */
  custom: boolean;
}

/**
 * The result emitted by the ServicePicker when the user chooses (or creates) a
 * service. It bundles everything the EntryEditor needs to start building an
 * entry: the `serviceRef` to persist, a display `name`, and the
 * `recommendedFields` to prefill. Catalog services carry their recommended
 * fields; custom services carry none (the user defines their own fields in the
 * editor), so `recommendedFields` is an empty array for them.
 */
export interface ServiceSelection {
  serviceRef: ServiceRef;
  /** Display name of the chosen service. */
  name: string;
  /** Whether the chosen service is a user-defined custom service. */
  custom: boolean;
  /** Recommended fields to prefill — empty for custom services. */
  recommendedFields: RecommendedField[];
}

// ---------------------------------------------------------------------------
// Password generator & settings
// ---------------------------------------------------------------------------

export interface PasswordGenOptions {
  length: number;
  upper: boolean;
  lower: boolean;
  digits: boolean;
  symbols: boolean;
}

/** Stored generator defaults (snake_case within the model's Settings). */
export interface PasswordGenDefaults {
  length: number;
  upper: boolean;
  lower: boolean;
  digits: boolean;
  symbols: boolean;
}

/** User settings, persisted encrypted inside the vault (model → snake_case). */
export interface Settings {
  /** Inactivity timeout before auto-lock, in seconds. `0` disables auto-lock. */
  auto_lock_seconds: number;
  lock_on_blur: boolean;
  clipboard_clear_seconds: number;
  password_gen_defaults: PasswordGenDefaults;
}

// ---------------------------------------------------------------------------
// Errors & UI
// ---------------------------------------------------------------------------

/** The single error surface returned across IPC (tagged by `code`, camelCase). */
export type KeyhavenError =
  | { code: "wrongCredentials" }
  | { code: "vaultCorrupted" }
  | { code: "incompatibleVersion" }
  | { code: "locked" }
  | { code: "io"; message: string }
  | { code: "invalidInput"; message: string };

export interface ToastMessage {
  id: number;
  kind: "info" | "success" | "warning" | "danger";
  text: string;
}
