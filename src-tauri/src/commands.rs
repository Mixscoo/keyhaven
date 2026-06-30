//! Tauri command handlers — the IPC surface.
//!
//! Each command is async/sync and returns a typed `Result<T, KeyhavenError>`.
//! No command ever returns the master password, recovery key, or derived key.
//!
//! Implemented so far:
//! - Session lifecycle (task 4.1): [`is_unlocked`], [`lock_vault`].
//! - Auto-lock (task 4.2): [`report_activity`] resets the backend inactivity
//!   deadline so the vault stays unlocked while the user is active.
//! - Vault creation (task 5.1): [`create_vault`] creates a new encrypted vault,
//!   optionally generating a recovery key (returned exactly once), and leaves
//!   the vault unlocked in the session.
//! - Unlock & password management (task 5.2): [`unlock_with_password`] and
//!   [`unlock_with_recovery_key`] open an existing vault into the session
//!   (returning a non-secret [`VaultSummary`]), and [`change_master_password`]
//!   rewrites only the password wrap + salt of the in-session vault.
//!
//! Planned (later tasks):
//! - Vault lifecycle: `vault_exists`.
//! - Entries: `list_entries`, `get_entry`, `create_entry`, `update_entry`,
//!   `delete_entry` — each gated through the session manager so they return
//!   [`KeyhavenError::Locked`] when the vault is locked.
//! - Services/catalog, utilities, and settings.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::catalog::{self, CatalogService};
use crate::crypto::KdfParams;
use crate::entries::{self, EntryInput};
use crate::error::KeyhavenError;
use crate::generator;
use crate::model::{CustomService, Entry, IconRef, Settings};
use crate::session::SessionManager;
use crate::vault;

/// Report whether a Keyhaven vault file already exists at `path`.
///
/// Used by the frontend's routing shell on startup to choose between the
/// first-run Setup screen (no vault) and the Unlock screen (vault present). This
/// is a non-secret filesystem existence check only — it does not read, decode,
/// or validate the file contents, so it never touches secret material and needs
/// no unlocked session. A `None`/empty path is treated as "no vault".
#[tauri::command]
pub fn vault_exists(path: Option<String>) -> Result<bool, KeyhavenError> {
    Ok(match path {
        Some(p) if !p.is_empty() => Path::new(&p).is_file(),
        _ => false,
    })
}

/// Report whether the vault is currently unlocked.
///
/// Used by the frontend on startup and after events to decide which screen to
/// show. Mirrors the design's `is_unlocked() -> Result<bool>` command.
#[tauri::command]
pub fn is_unlocked(session: State<'_, Arc<SessionManager>>) -> Result<bool, KeyhavenError> {
    Ok(session.is_unlocked())
}

/// Lock the vault: drop and zeroize the decrypted model and VEK from memory and
/// emit the `vault-locked` event so the frontend returns to the Unlock screen
/// (Req 3.5, 15.5).
///
/// Locking an already-locked vault is a harmless no-op that still emits the
/// event, keeping the frontend's routing deterministic.
#[tauri::command]
pub fn lock_vault(
    app: AppHandle,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    session.lock(&app);
    Ok(())
}

/// Report qualifying user activity, resetting the backend auto-lock inactivity
/// countdown (Req 4.4).
///
/// The frontend calls this (debounced) on user interaction. The authoritative
/// timer lives in the backend, so this only nudges the deadline forward; it can
/// never keep a locked vault open. A no-op while locked.
#[tauri::command]
pub fn report_activity(session: State<'_, Arc<SessionManager>>) -> Result<(), KeyhavenError> {
    session.report_activity();
    Ok(())
}

/// Result of [`create_vault`].
///
/// Carries the freshly generated recovery key **only when** one was requested
/// (Req 2.2). The key is exposed here exactly once — it is never persisted in a
/// recoverable form and there is no command to retrieve it again — so the
/// frontend is responsible for the one-time reveal + "I saved it" gate
/// (task 10.1). Serializes with a camelCase `recoveryKey` field matching the
/// frontend contract, and the field is omitted entirely when absent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVaultResult {
    /// The one-time recovery key, present only when `generate_recovery` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_key: Option<String>,
}

/// Create a brand-new encrypted vault at `path`, protected by `master_password`
/// and (optionally) a freshly generated recovery key, then open it unlocked.
///
/// Implements the design's create flow at the command layer (Req 1.1, 1.2, 2.1,
/// 2.2, 2.6):
/// - Derives the master key with Argon2id over a fresh random salt, wraps a
///   random VEK under it, encrypts the (empty) payload, and writes the file
///   atomically — all handled by [`vault::create_vault`].
/// - When `generate_recovery` is `true`, a cryptographically random recovery key
///   is generated and a second, independent wrap of the VEK is stored, so either
///   secret can later unlock the vault. The recovery key is returned **once** in
///   [`CreateVaultResult`]. When `false`, the vault is created password-only
///   (Req 2.6) and no recovery key is returned.
/// - On success the vault is installed in the session as the current unlocked
///   vault (Req 1.7), so the frontend transitions straight into the unlocked UI.
///
/// The master password is held in a [`Zeroizing`] buffer and wiped as soon as
/// key derivation completes; it is never echoed back.
///
/// Note: the confirm-password-twice check (Req 1.2/1.3) is enforced in the UI
/// before this command is called — the backend receives a single password.
#[tauri::command]
pub fn create_vault(
    master_password: String,
    generate_recovery: bool,
    path: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<CreateVaultResult, KeyhavenError> {
    create_vault_impl(
        &session,
        master_password,
        generate_recovery,
        Path::new(&path),
        KdfParams::recommended(),
    )
}

/// Inner implementation of [`create_vault`], parameterized over the
/// [`SessionManager`] and [`KdfParams`] so it can be unit-tested without a Tauri
/// runtime and with fast KDF parameters. The public command is a thin wrapper
/// that supplies the Tauri-managed session and the production KDF parameters.
fn create_vault_impl(
    session: &SessionManager,
    master_password: String,
    generate_recovery: bool,
    path: &Path,
    params: KdfParams,
) -> Result<CreateVaultResult, KeyhavenError> {
    if master_password.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "master password must not be empty".to_string(),
        });
    }
    if path.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "vault path must not be empty".to_string(),
        });
    }

    // Reuse the String's buffer as the secret bytes and wipe them after the
    // create flow has derived the master key (Req 1.8, 15.5).
    let password = Zeroizing::new(master_password.into_bytes());

    let (vault, recovery_key) =
        vault::create_vault(path, password.as_slice(), generate_recovery, params)?;

    // Leave the vault unlocked after creation (Req 1.7), wiring into the session
    // manager so subsequent commands operate on the open vault and the auto-lock
    // deadline is armed.
    session.set_unlocked(vault);

    Ok(CreateVaultResult { recovery_key })
}

/// Lightweight, non-secret summary of an unlocked vault, returned by the unlock
/// commands so the frontend can render the unlocked shell and decide its next
/// route. It never carries credential values, the master password, the recovery
/// key, or the derived key.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSummary {
    /// Whether this vault has a recovery-key section configured.
    pub has_recovery: bool,
    /// Number of entries currently stored (non-secret count only).
    pub entry_count: usize,
    /// `true` when the vault was opened via the recovery key. The frontend uses
    /// this to route the user to "set a new master password" (Req 2.7).
    pub unlocked_via_recovery: bool,
}

/// Build a [`VaultSummary`] from an open vault. `via_recovery` records whether
/// the unlock used the recovery key (so the UI can prompt for a new master
/// password per Req 2.7).
fn summarize(vault: &vault::OpenVault, via_recovery: bool) -> VaultSummary {
    VaultSummary {
        has_recovery: vault.has_recovery(),
        entry_count: vault.model().entries.len(),
        unlocked_via_recovery: via_recovery,
    }
}

/// Unlock an existing vault at `path` with the master password and install it as
/// the current unlocked session (Req 3.1, 3.2).
///
/// Derives the master key with Argon2id over the header's `MASTER_SALT` and
/// unwraps the VEK from `PW_WRAP`; an incorrect password fails the AEAD
/// authentication tag and surfaces as [`KeyhavenError::WrongCredentials`]
/// (Req 3.3) — no usable VEK is ever produced (Property 4). A damaged file
/// surfaces as [`KeyhavenError::VaultCorrupted`] (Req 3.6). The KDF parameters
/// are read from the file header, so no parameters are passed here.
///
/// The master password is held in a [`Zeroizing`] buffer and wiped once the
/// unlock completes; it is never echoed back. Returns a non-secret
/// [`VaultSummary`].
#[tauri::command]
pub fn unlock_with_password(
    master_password: String,
    path: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<VaultSummary, KeyhavenError> {
    unlock_with_password_impl(&session, master_password, Path::new(&path))
}

/// Inner implementation of [`unlock_with_password`], parameterized over the
/// [`SessionManager`] so it can be unit-tested without a Tauri runtime.
fn unlock_with_password_impl(
    session: &SessionManager,
    master_password: String,
    path: &Path,
) -> Result<VaultSummary, KeyhavenError> {
    if master_password.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "master password must not be empty".to_string(),
        });
    }
    if path.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "vault path must not be empty".to_string(),
        });
    }

    let password = Zeroizing::new(master_password.into_bytes());
    let vault = vault::unlock_with_password(path, password.as_slice())?;

    let summary = summarize(&vault, false);
    session.set_unlocked(vault);
    Ok(summary)
}

/// Unlock an existing vault at `path` with its recovery key and install it as the
/// current unlocked session (Req 2.7).
///
/// Behaves like [`unlock_with_password`] but derives from the recovery section
/// (`REC_SALT` / `REC_WRAP`). A wrong recovery key — or a recovery key supplied
/// for a vault that has no recovery section — surfaces as
/// [`KeyhavenError::WrongCredentials`] (Req 2.8, 3.3). The returned
/// [`VaultSummary`] has `unlocked_via_recovery = true` so the frontend prompts
/// the user to set a new master password (Req 2.7), which it does via
/// [`change_master_password`] supplying the recovery key as the current secret.
#[tauri::command]
pub fn unlock_with_recovery_key(
    recovery_key: String,
    path: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<VaultSummary, KeyhavenError> {
    unlock_with_recovery_key_impl(&session, recovery_key, Path::new(&path))
}

/// Inner implementation of [`unlock_with_recovery_key`], parameterized over the
/// [`SessionManager`] for unit testing without a Tauri runtime.
fn unlock_with_recovery_key_impl(
    session: &SessionManager,
    recovery_key: String,
    path: &Path,
) -> Result<VaultSummary, KeyhavenError> {
    if recovery_key.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "recovery key must not be empty".to_string(),
        });
    }
    if path.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "vault path must not be empty".to_string(),
        });
    }

    let key = Zeroizing::new(recovery_key.into_bytes());
    let vault = vault::unlock_with_recovery_key(path, key.as_slice())?;

    let summary = summarize(&vault, true);
    session.set_unlocked(vault);
    Ok(summary)
}

/// Change the master password of the currently unlocked vault (Req 3.2).
///
/// Operates on the in-session vault (no `path` argument, per the design command
/// surface): the VEK is already held in memory, so only the password wrap and
/// its salt are rewritten — the payload ciphertext and the recovery wrap are
/// left untouched (Property 6). The change is persisted atomically to the file
/// the vault was opened from.
///
/// `current` must prove the caller can already open this vault: it is accepted
/// as the current master password **or**, in the recovery flow (Req 2.7), the
/// recovery key the user just unlocked with. A secret matching neither is
/// rejected with [`KeyhavenError::WrongCredentials`] (Req 3.3). Returns
/// [`KeyhavenError::Locked`] if no vault is unlocked.
///
/// Both secrets are held in [`Zeroizing`] buffers and wiped when this returns.
#[tauri::command]
pub fn change_master_password(
    current: String,
    new_password: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    change_master_password_impl(&session, current, new_password)
}

/// Inner implementation of [`change_master_password`], parameterized over the
/// [`SessionManager`] for unit testing without a Tauri runtime.
fn change_master_password_impl(
    session: &SessionManager,
    current: String,
    new_password: String,
) -> Result<(), KeyhavenError> {
    if current.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "current secret must not be empty".to_string(),
        });
    }
    if new_password.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "new master password must not be empty".to_string(),
        });
    }

    let current = Zeroizing::new(current.into_bytes());
    let new_password = Zeroizing::new(new_password.into_bytes());

    // `with_vault_mut` gates on the unlocked state (returns `Locked` otherwise);
    // the inner repository call returns a `VaultRepoError` that maps onto the
    // IPC-facing `KeyhavenError` (e.g. `WrongCredentials`).
    session
        .with_vault_mut(|v| v.change_master_password(current.as_slice(), new_password.as_slice()))?
        .map_err(KeyhavenError::from)
}

/// The result of a create/update entry operation: the saved entry plus a
/// non-blocking warning flag.
///
/// `emptyWarning` is `true` when the saved entry has no filled-in content
/// (Req 5.6, 8.6). Saving is **not** blocked in that case — the flag lets the
/// frontend surface a gentle warning while still persisting the entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySaveResult {
    /// The entry as stored (with its server-assigned id and timestamps).
    pub entry: Entry,
    /// Whether the saved entry is empty and the UI should warn the user.
    pub empty_warning: bool,
}

/// List entries from the unlocked vault as lightweight, non-secret summaries,
/// filtered by an optional search `query` and paginated by an optional `page`
/// (Req 9.1, 9.2, 9.3, 9.4).
///
/// Search matches the service name/ref, field labels, and **non-secret** field
/// values; secret values are never indexed or returned (Property 7). The result
/// carries the page slice plus the total match count so the frontend can drive a
/// virtualized/paged list. Gated through the session, so it returns
/// [`KeyhavenError::Locked`] when the vault is locked.
#[tauri::command]
pub fn list_entries(
    query: Option<String>,
    page: Option<entries::Page>,
    session: State<'_, Arc<SessionManager>>,
) -> Result<entries::EntryList, KeyhavenError> {
    list_entries_impl(&session, query, page)
}

fn list_entries_impl(
    session: &SessionManager,
    query: Option<String>,
    page: Option<entries::Page>,
) -> Result<entries::EntryList, KeyhavenError> {
    session.with_vault(|v| entries::list_entries(v.model(), query, page))
}

/// Fetch a single entry by id from the unlocked vault (Req 8.2).
///
/// Returns [`KeyhavenError::Locked`] when the vault is locked and
/// [`KeyhavenError::InvalidInput`] when no entry with that id exists.
#[tauri::command]
pub fn get_entry(
    id: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<Entry, KeyhavenError> {
    get_entry_impl(&session, &id)
}

fn get_entry_impl(session: &SessionManager, id: &str) -> Result<Entry, KeyhavenError> {
    session.with_vault(|v| {
        entries::get_entry(v.model(), id)
            .cloned()
            .ok_or(KeyhavenError::InvalidInput {
                message: "no entry with that id exists".to_string(),
            })
    })?
}

/// Create a new entry in the unlocked vault and persist it via the atomic write
/// path (Req 5.3, 5.4, 5.5, 7.2, 7.5).
///
/// Assigns a UUID and creation/update timestamps, then writes the updated vault
/// to disk atomically. Returns the stored entry together with the empty-entry
/// warning flag (Req 5.6). Returns [`KeyhavenError::Locked`] when locked.
#[tauri::command]
pub fn create_entry(
    input: EntryInput,
    session: State<'_, Arc<SessionManager>>,
) -> Result<EntrySaveResult, KeyhavenError> {
    create_entry_impl(&session, input)
}

fn create_entry_impl(
    session: &SessionManager,
    input: EntryInput,
) -> Result<EntrySaveResult, KeyhavenError> {
    session.with_vault_mut(|v| {
        let entry = entries::create_entry(v.model_mut(), input, entries::now_iso());
        let empty_warning = entries::entry_is_empty(&entry);
        let path = v.path().to_path_buf();
        v.save(&path).map_err(KeyhavenError::from)?;
        Ok::<EntrySaveResult, KeyhavenError>(EntrySaveResult {
            entry,
            empty_warning,
        })
    })?
}

/// Update an existing entry in the unlocked vault and persist it atomically
/// (Req 7.3, 7.6, 8.5).
///
/// Preserves the entry id and `created_at`, refreshes `updated_at`, and replaces
/// the field set with the supplied input. Returns [`KeyhavenError::InvalidInput`]
/// if the id is unknown and [`KeyhavenError::Locked`] when locked.
#[tauri::command]
pub fn update_entry(
    id: String,
    input: EntryInput,
    session: State<'_, Arc<SessionManager>>,
) -> Result<EntrySaveResult, KeyhavenError> {
    update_entry_impl(&session, &id, input)
}

fn update_entry_impl(
    session: &SessionManager,
    id: &str,
    input: EntryInput,
) -> Result<EntrySaveResult, KeyhavenError> {
    session.with_vault_mut(|v| {
        let entry = entries::update_entry(v.model_mut(), id, input, entries::now_iso())
            .map_err(KeyhavenError::from)?;
        let empty_warning = entries::entry_is_empty(&entry);
        let path = v.path().to_path_buf();
        v.save(&path).map_err(KeyhavenError::from)?;
        Ok::<EntrySaveResult, KeyhavenError>(EntrySaveResult {
            entry,
            empty_warning,
        })
    })?
}

/// Delete an entry by id from the unlocked vault and persist atomically
/// (Req 8.6).
///
/// Deletion confirmation is enforced at the UI layer; the backend removes the
/// entry unconditionally. Returns [`KeyhavenError::InvalidInput`] if the id is
/// unknown and [`KeyhavenError::Locked`] when locked.
#[tauri::command]
pub fn delete_entry(
    id: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    delete_entry_impl(&session, &id)
}

fn delete_entry_impl(session: &SessionManager, id: &str) -> Result<(), KeyhavenError> {
    session.with_vault_mut(|v| {
        entries::delete_entry(v.model_mut(), id).map_err(KeyhavenError::from)?;
        let path = v.path().to_path_buf();
        v.save(&path).map_err(KeyhavenError::from)?;
        Ok::<(), KeyhavenError>(())
    })?
}

// ===========================================================================
// Task 7.2 — service catalog & custom services
// ===========================================================================

/// Search the bundled, offline service catalog by name and aliases (Req 6.1,
/// 12.2).
///
/// The catalog is static, non-secret data embedded in the binary, so this
/// command is **not** gated on an unlocked vault — the service picker can browse
/// and search it before (or independent of) any vault being open. An empty/blank
/// `query` returns the entire catalog (see [`catalog::search_catalog`]).
#[tauri::command]
pub fn search_catalog(query: String) -> Result<Vec<CatalogService>, KeyhavenError> {
    search_catalog_impl(&query)
}

/// Inner implementation of [`search_catalog`], kept as a pure function over the
/// catalog so it is unit-testable without a Tauri runtime. Catalog search needs
/// no session gating (the catalog holds no secrets).
fn search_catalog_impl(query: &str) -> Result<Vec<CatalogService>, KeyhavenError> {
    Ok(catalog::search_catalog(query))
}

/// A custom service as returned to the frontend, with an explicit `custom: true`
/// indicator flag (Req 6.3).
///
/// The persisted [`CustomService`] model already marks a service as custom by
/// virtue of living in `VaultModel.custom_services`; rather than polluting the
/// stored model with a redundant flag, the wire shape adds `custom: true` here
/// so the UI can render a "Custom" badge uniformly. Serializes in camelCase to
/// match the other command result types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomServiceView {
    /// Stable custom-service id (UUID string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// The service's icon (built-in reference or inline data).
    pub icon: IconRef,
    /// Always `true`: marks this as a user-defined custom service (Req 6.3).
    pub custom: bool,
}

impl From<&CustomService> for CustomServiceView {
    fn from(service: &CustomService) -> Self {
        CustomServiceView {
            id: service.id.clone(),
            name: service.name.clone(),
            icon: service.icon.clone(),
            custom: true,
        }
    }
}

/// List the user-defined custom services stored in the unlocked vault (Req 6.5).
///
/// Gated through the session, so it returns [`KeyhavenError::Locked`] when the
/// vault is locked. Each result carries the `custom: true` indicator flag
/// (Req 6.3).
#[tauri::command]
pub fn list_custom_services(
    session: State<'_, Arc<SessionManager>>,
) -> Result<Vec<CustomServiceView>, KeyhavenError> {
    list_custom_services_impl(&session)
}

fn list_custom_services_impl(
    session: &SessionManager,
) -> Result<Vec<CustomServiceView>, KeyhavenError> {
    session.with_vault(|v| {
        v.model()
            .custom_services
            .iter()
            .map(CustomServiceView::from)
            .collect()
    })
}

/// Create a user-defined custom service with a custom icon, persisted in the
/// unlocked vault (Req 6.2, 6.4).
///
/// Validates that `name` is non-empty (else [`KeyhavenError::InvalidInput`]),
/// mints a fresh UUID v4 id, appends the [`CustomService`] to the vault model's
/// `custom_services`, and persists the vault atomically (same write path as the
/// entry commands). Returns the created service with its `custom: true`
/// indicator (Req 6.3). Returns [`KeyhavenError::Locked`] when the vault is
/// locked.
#[tauri::command]
pub fn create_custom_service(
    name: String,
    icon: IconRef,
    session: State<'_, Arc<SessionManager>>,
) -> Result<CustomServiceView, KeyhavenError> {
    create_custom_service_impl(&session, name, icon)
}

fn create_custom_service_impl(
    session: &SessionManager,
    name: String,
    icon: IconRef,
) -> Result<CustomServiceView, KeyhavenError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "service name must not be empty".to_string(),
        });
    }

    session.with_vault_mut(|v| {
        let service = CustomService {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            icon,
        };
        v.model_mut().custom_services.push(service.clone());
        let path = v.path().to_path_buf();
        v.save(&path).map_err(KeyhavenError::from)?;
        Ok::<CustomServiceView, KeyhavenError>(CustomServiceView::from(&service))
    })?
}

// ===========================================================================
// Task 8.1 — password generator
// ===========================================================================

/// Generate a strong random password from the supplied options (Req 10.1, 10.2,
/// 10.3).
///
/// This is a pure utility that draws from the OS CSPRNG (Req 10.3) and holds no
/// vault state, so it is **not** gated on an unlocked session — the inline
/// generator in the entry editor and the settings preview can both call it
/// independently of any open vault. The generated password is built in a
/// zeroizing buffer in the generator core; this command returns it to the
/// frontend, which places it into the target field and offers regenerate/accept
/// (Req 10.4, handled in the UI).
///
/// Returns [`KeyhavenError::InvalidInput`] when the options cannot produce a
/// valid password (no character set selected, or an out-of-range / too-small
/// length); see [`generator::generate_password`].
#[tauri::command]
pub fn generate_password(
    opts: generator::PasswordGenOptions,
) -> Result<String, KeyhavenError> {
    generator::generate_password(opts)
}

// ===========================================================================
// Task 8.2 — clipboard copy with auto-clear
// ===========================================================================

/// Copy the value of a secret field to the system clipboard and schedule a
/// backend task to clear it after `clipboard_clear_seconds` (Req 8.3, 8.4).
///
/// The value is looked up from the **unlocked** session, so the command is gated
/// through [`SessionManager::with_vault`] and returns [`KeyhavenError::Locked`]
/// when the vault is locked. An unknown `entry_id`/`field_id` surfaces as
/// [`KeyhavenError::InvalidInput`].
///
/// ## Clear-if-unchanged
///
/// After the configured delay, the scheduled task clears the clipboard **only
/// if it still holds the exact value we copied** (see [`should_clear_clipboard`]).
/// This avoids clobbering unrelated content the user copied in the meantime —
/// if they've copied something else, Keyhaven leaves it alone.
///
/// A `clipboard_clear_seconds` of `0` disables the auto-clear (the value is
/// copied but never automatically wiped), mirroring the "disabled" convention
/// used elsewhere in settings.
#[tauri::command]
pub fn copy_secret_to_clipboard(
    app: AppHandle,
    entry_id: String,
    field_id: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    copy_secret_to_clipboard_impl(&app, &session, &entry_id, &field_id)
}

/// Inner implementation of [`copy_secret_to_clipboard`], parameterized over a
/// [`ClipboardSink`] so the copy/schedule wiring can be driven without a real
/// Tauri clipboard. The public command supplies the Tauri [`AppHandle`] (which
/// implements [`ClipboardSink`] via the clipboard-manager plugin).
fn copy_secret_to_clipboard_impl<C: ClipboardSink>(
    clipboard: &C,
    session: &SessionManager,
    entry_id: &str,
    field_id: &str,
) -> Result<(), KeyhavenError> {
    // Look up the value (and the configured clear delay) from the unlocked
    // session. The outer `?` propagates the `Locked` gate; the inner `?`
    // propagates a missing entry/field.
    let (value, clear_seconds) =
        session.with_vault(|v| lookup_field_value(v.model(), entry_id, field_id))??;

    clipboard
        .write_text(&value)
        .map_err(|message| KeyhavenError::Io { message })?;

    // Arm the clear-if-unchanged task unless auto-clear is disabled (0).
    if clear_seconds > 0 {
        clipboard.schedule_clear(value, clear_seconds);
    }
    Ok(())
}

/// Resolve the value of the field identified by `entry_id`/`field_id` together
/// with the vault's configured clipboard-clear delay, from the decrypted model.
///
/// Returns [`KeyhavenError::InvalidInput`] when the entry or field does not
/// exist. Kept as a pure function over a borrowed model so it is unit-testable
/// without a session or Tauri runtime.
fn lookup_field_value(
    model: &crate::model::VaultModel,
    entry_id: &str,
    field_id: &str,
) -> Result<(String, u32), KeyhavenError> {
    let entry = entries::get_entry(model, entry_id).ok_or(KeyhavenError::InvalidInput {
        message: "no entry with that id exists".to_string(),
    })?;
    let field = entry
        .fields
        .iter()
        .find(|f| f.id == field_id)
        .ok_or(KeyhavenError::InvalidInput {
            message: "no field with that id exists".to_string(),
        })?;
    Ok((
        field.value.clone(),
        model.settings.clipboard_clear_seconds,
    ))
}

/// The clear-if-unchanged decision: clear the clipboard only when it still holds
/// the exact value Keyhaven copied (Req 8.4).
///
/// `current` is the clipboard's present text (`None` if it could not be read or
/// holds non-text content). Returns `true` only when it equals `copied`, so any
/// content the user copied since — or an unreadable clipboard — is left
/// untouched. Pure and side-effect-free for unit testing.
fn should_clear_clipboard(current: Option<&str>, copied: &str) -> bool {
    current == Some(copied)
}

/// Abstraction over the system clipboard so the copy/schedule logic can be
/// tested without a Tauri runtime.
///
/// Production uses the Tauri [`AppHandle`] (clipboard-manager plugin); tests use
/// a lightweight in-memory fake. `write_text` reports failures as a plain
/// `String` message (mapped to [`KeyhavenError::Io`] by the caller), and
/// `schedule_clear` arms the delayed clear-if-unchanged task.
trait ClipboardSink {
    /// Place `text` on the system clipboard.
    fn write_text(&self, text: &str) -> Result<(), String>;
    /// Schedule a task that, after `clear_seconds`, clears the clipboard only if
    /// it still contains `copied` (see [`should_clear_clipboard`]).
    fn schedule_clear(&self, copied: String, clear_seconds: u32);
}

impl ClipboardSink for AppHandle {
    fn write_text(&self, text: &str) -> Result<(), String> {
        // On Windows, write the text together with markers that exclude it from
        // Clipboard History (Win+V) and Cloud Clipboard, so a copied secret never
        // lingers in those OS-level stores. Other platforms use the plugin.
        #[cfg(windows)]
        {
            let _ = self;
            return set_clipboard_excluded(text);
        }
        #[cfg(not(windows))]
        {
            use tauri_plugin_clipboard_manager::ClipboardExt;
            self.clipboard()
                .write_text(text.to_string())
                .map_err(|e| format!("clipboard write failed: {e}"))
        }
    }

    fn schedule_clear(&self, copied: String, clear_seconds: u32) {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let app = self.clone();
        // Run off the main thread: the clipboard read may block/deadlock on the
        // main thread on some platforms, and we must not stall the timer there.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(clear_seconds as u64));
            // Read the current clipboard text; treat unreadable / non-text as
            // "changed" so we never clear content that isn't ours.
            let current = app.clipboard().read_text().ok();
            if should_clear_clipboard(current.as_deref(), &copied) {
                let _ = app.clipboard().clear();
            }
        });
    }
}

/// Windows: place `text` on the clipboard and tag it so the OS excludes it from
/// Clipboard History (Win+V) and Cloud Clipboard.
///
/// All operations run inside a **single** clipboard session (open → set text →
/// add exclusion markers → close on drop), because Windows snapshots the
/// clipboard when the session closes — the markers must already be present then,
/// or history/cloud would capture the secret before we could opt out.
///
/// The three formats are the documented opt-outs: the mere presence of
/// `ExcludeClipboardContentFromMonitorProcessing` signals "sensitive, don't
/// monitor", and `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard`
/// set to a `0` DWORD explicitly disable history capture and cloud upload.
///
/// IMPORTANT: the markers are written with [`raw::set_without_clear`], NOT
/// [`raw::set`]. `raw::set` calls `EmptyClipboard()` before every write, so
/// using it for the markers would wipe the Unicode text we just placed — the
/// clipboard would end up holding only a marker and no pastable text. Writing
/// the text first (which clears once) and then *appending* the markers without
/// clearing keeps both on the clipboard together.
#[cfg(windows)]
fn set_clipboard_excluded(text: &str) -> Result<(), String> {
    use clipboard_win::{formats, raw, register_format, Clipboard, Setter};

    let _clip = Clipboard::new_attempts(10)
        .map_err(|e| format!("could not open the clipboard: {e}"))?;

    // Writes CF_UNICODETEXT; this clears the clipboard once before setting.
    formats::Unicode
        .write_clipboard(&text)
        .map_err(|e| format!("clipboard write failed: {e}"))?;

    // Append the exclusion markers WITHOUT clearing, so the text above survives.
    if let Some(fmt) = register_format("ExcludeClipboardContentFromMonitorProcessing") {
        let _ = raw::set_without_clear(fmt.get(), &[0u8; 4]);
    }
    if let Some(fmt) = register_format("CanIncludeInClipboardHistory") {
        let _ = raw::set_without_clear(fmt.get(), &0u32.to_ne_bytes());
    }
    if let Some(fmt) = register_format("CanUploadToCloudClipboard") {
        let _ = raw::set_without_clear(fmt.get(), &0u32.to_ne_bytes());
    }
    Ok(())
}

// ===========================================================================
// Task 8.3 — settings persistence and vault export/import
// ===========================================================================

/// Return the current user settings from the unlocked vault (Req 4.3, 8.4).
///
/// Settings are stored **inside** the encrypted vault model, so reading them
/// requires an unlocked session: this is gated through
/// [`SessionManager::with_vault`] and returns [`KeyhavenError::Locked`] when the
/// vault is locked.
#[tauri::command]
pub fn get_settings(session: State<'_, Arc<SessionManager>>) -> Result<Settings, KeyhavenError> {
    get_settings_impl(&session)
}

/// Inner implementation of [`get_settings`], parameterized over the
/// [`SessionManager`] so it is unit-testable without a Tauri runtime.
fn get_settings_impl(session: &SessionManager) -> Result<Settings, KeyhavenError> {
    session.with_vault(|v| v.model().settings.clone())
}

/// Persist updated user settings into the encrypted vault and apply their
/// runtime effects (Req 4.3, 8.4).
///
/// Writes the new [`Settings`] into the unlocked model, persists the vault via
/// the atomic write path, then re-arms the auto-lock countdown so a changed
/// `auto_lock_seconds` takes effect immediately instead of waiting for the next
/// unlock (task 4.2, Req 4.3). `lock_on_blur` is read live from the model on each
/// window-blur event, so it needs no explicit refresh; `clipboard_clear_seconds`
/// and the generator defaults are read on demand. Returns
/// [`KeyhavenError::Locked`] when the vault is locked.
#[tauri::command]
pub fn update_settings(
    settings: Settings,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    update_settings_impl(&session, settings)
}

/// Inner implementation of [`update_settings`], parameterized over the
/// [`SessionManager`] so it is unit-testable without a Tauri runtime.
fn update_settings_impl(
    session: &SessionManager,
    settings: Settings,
) -> Result<(), KeyhavenError> {
    session.with_vault_mut(|v| {
        v.model_mut().settings = settings;
        let path = v.path().to_path_buf();
        v.save(&path).map_err(KeyhavenError::from)
    })??;

    // Re-apply the (possibly changed) auto-lock timeout to the live countdown so
    // the new duration is effective immediately (Req 4.3).
    session.refresh_auto_lock_from_settings();
    Ok(())
}

/// Export the current vault as a single, portable, still-encrypted file to
/// `destination` (Req 11.1, 11.2).
///
/// The vault is exported by copying the live encrypted vault file on disk. Every
/// command mutation persists immediately through the atomic write path, so the
/// on-disk file already reflects the current state. The exported copy stays fully
/// encrypted and therefore still requires the master password or recovery key to
/// open (Req 11.2; Property 2) — no plaintext credential data is ever written.
///
/// Gated through the session so the source path is known and the caller has
/// proven access: returns [`KeyhavenError::Locked`] when the vault is locked.
#[tauri::command]
pub fn export_vault(
    destination: String,
    session: State<'_, Arc<SessionManager>>,
) -> Result<(), KeyhavenError> {
    export_vault_impl(&session, Path::new(&destination))
}

/// Inner implementation of [`export_vault`], parameterized over the
/// [`SessionManager`] so it is unit-testable without a Tauri runtime.
fn export_vault_impl(session: &SessionManager, destination: &Path) -> Result<(), KeyhavenError> {
    if destination.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "export destination must not be empty".to_string(),
        });
    }

    // The source is the live vault file the session was opened from.
    let source = session.with_vault(|v| v.path().to_path_buf())?;

    // Refuse to export onto the live vault file itself: copying a file onto its
    // own path would truncate and corrupt it.
    if is_same_existing_file(&source, destination) {
        return Err(KeyhavenError::InvalidInput {
            message: "export destination must differ from the current vault file".to_string(),
        });
    }

    std::fs::copy(&source, destination).map_err(|e| KeyhavenError::Io {
        message: format!("could not write export to destination: {e}"),
    })?;
    Ok(())
}

/// Whether `a` and `b` refer to the same existing file on disk (compared by
/// canonical path). Returns `false` when `b` does not yet exist — the common
/// export-to-a-new-file case — so a fresh export is never mistaken for a
/// self-overwrite.
fn is_same_existing_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Write the one-time recovery key to a user-chosen plaintext file.
///
/// The recovery key is generated and shown exactly once at vault creation
/// (Req 2.2). This convenience command lets the user save it straight to a file
/// via the native save dialog instead of copying it by hand. The key is supplied
/// by the frontend (the same value it just displayed) — the backend does not
/// persist or echo it anywhere else. The file is written with a short
/// human-readable header so the saved file is self-explanatory.
///
/// This deliberately writes a sensitive value in plaintext because the user has
/// explicitly chosen to export it; it mirrors the dialog-scoped file-write the
/// frontend already uses for [`export_vault`]. Returns
/// [`KeyhavenError::InvalidInput`] for an empty destination or key, and
/// [`KeyhavenError::Io`] on a write failure.
#[tauri::command]
pub fn save_recovery_key(
    destination: String,
    recovery_key: String,
) -> Result<(), KeyhavenError> {
    save_recovery_key_impl(Path::new(&destination), &recovery_key)
}

/// Inner implementation of [`save_recovery_key`], kept as a pure function over
/// the destination path and key so it is unit-testable without a Tauri runtime.
fn save_recovery_key_impl(destination: &Path, recovery_key: &str) -> Result<(), KeyhavenError> {
    if destination.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "save destination must not be empty".to_string(),
        });
    }
    if recovery_key.trim().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "recovery key must not be empty".to_string(),
        });
    }

    std::fs::write(destination, format_recovery_key_file(recovery_key)).map_err(|e| {
        KeyhavenError::Io {
            message: format!("could not write the recovery key file: {e}"),
        }
    })?;
    Ok(())
}

/// Render the self-explanatory text content of a saved recovery-key file. Uses
/// CRLF line endings so the file reads correctly in Windows Notepad as well as
/// every other common editor.
fn format_recovery_key_file(recovery_key: &str) -> String {
    let generated = chrono::Utc::now().to_rfc3339();
    [
        "Keyhaven Recovery Key",
        "=====================",
        "",
        "Keep this key private and safe. Anyone who has it can unlock your vault.",
        "Store it somewhere separate from this computer.",
        "",
        "This key is the ONLY way back into your vault if you forget your master",
        "password. Keyhaven cannot recover it for you.",
        "",
        "Recovery key:",
        recovery_key,
        "",
        &format!("Generated: {generated}"),
    ]
    .join("\r\n")
        + "\r\n"
}

/// Validate that `source` is a readable Keyhaven vault **before** anything else
/// happens, without altering any existing vault (Req 11.6; Property 9).
///
/// Reads the file and decodes its binary header via the vault-format layer
/// ([`crate::vault::decode`]), which validates `MAGIC` and `FORMAT_VER`: an
/// unknown/foreign magic (or a structurally invalid file) surfaces as
/// [`KeyhavenError::VaultCorrupted`] and a newer-than-supported version as
/// [`KeyhavenError::IncompatibleVersion`]. This command performs **no writes**,
/// so a failed, foreign, or incompatible import cannot corrupt or modify the
/// current vault (Property 9). It is intentionally **not** gated on an unlocked
/// session, since import is typically performed from the unlock screen on a new
/// device.
///
/// On success the imported file is confirmed to be a valid vault; the frontend
/// then treats `source` as the active vault path and unlocks it with the master
/// password or recovery key (Req 11.3, 11.4) via the existing unlock commands.
#[tauri::command]
pub fn import_vault(source: String) -> Result<(), KeyhavenError> {
    import_vault_impl(Path::new(&source))
}

/// Inner implementation of [`import_vault`], kept as a pure function over the
/// source path so it is unit-testable without a Tauri runtime.
fn import_vault_impl(source: &Path) -> Result<(), KeyhavenError> {
    if source.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "import source must not be empty".to_string(),
        });
    }

    let bytes = std::fs::read(source).map_err(|e| KeyhavenError::Io {
        message: format!("could not read import file: {e}"),
    })?;

    // Validate magic + format version (and overall structure) before touching
    // anything. Route the format error through the repository error's existing
    // conversion so it lands on the right IPC error code.
    crate::vault::decode(&bytes)
        .map_err(|e| KeyhavenError::from(crate::vault::VaultRepoError::from(e)))?;
    Ok(())
}

/// Install an external vault file as **this device's** vault, for the first-run
/// "I already have a vault" flow (Req 11.3/11.4).
///
/// Validates that `source` is a real Keyhaven vault, then copies it to
/// `destination` (the device's default vault path) so future opens and saves use
/// the managed location. It deliberately **refuses to overwrite an existing
/// vault** at `destination` — this flow is only for a device that has no vault
/// yet, and we never want an import to clobber someone's data.
///
/// No secret is involved: the copied file stays fully encrypted, so after the
/// copy the frontend routes to the Unlock screen where the user enters the
/// vault's **original** master password (or recovery key) to open it. Returns
/// [`KeyhavenError::VaultCorrupted`]/[`KeyhavenError::IncompatibleVersion`] for a
/// bad source, [`KeyhavenError::InvalidInput`] when a vault already exists or the
/// paths are empty/identical, and [`KeyhavenError::Io`] on a copy failure.
#[tauri::command]
pub fn import_external_vault(
    source: String,
    destination: String,
) -> Result<(), KeyhavenError> {
    import_external_vault_impl(Path::new(&source), Path::new(&destination))
}

/// Inner implementation of [`import_external_vault`], a pure function over the
/// two paths so it is unit-testable without a Tauri runtime.
fn import_external_vault_impl(source: &Path, destination: &Path) -> Result<(), KeyhavenError> {
    if source.as_os_str().is_empty() || destination.as_os_str().is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "import source and destination must not be empty".to_string(),
        });
    }

    // Confirm the source is a genuine Keyhaven vault before any disk write.
    import_vault_impl(source)?;

    // Never clobber an existing vault on this device.
    if destination.is_file() {
        return Err(KeyhavenError::InvalidInput {
            message: "a vault already exists on this device; importing would overwrite it"
                .to_string(),
        });
    }
    if is_same_existing_file(source, destination) {
        return Err(KeyhavenError::InvalidInput {
            message: "the import source and destination are the same file".to_string(),
        });
    }

    if let Some(dir) = destination.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| KeyhavenError::Io {
                message: format!("could not create the vault folder: {e}"),
            })?;
        }
    }

    std::fs::copy(source, destination).map_err(|e| KeyhavenError::Io {
        message: format!("could not import the vault file: {e}"),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::unlock_with_password;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Cheap Argon2id parameters so the command tests stay fast while still
    /// exercising the real create/unlock crypto path.
    fn fast_params() -> KdfParams {
        KdfParams {
            m_cost: 512,
            t_cost: 1,
            p_cost: 1,
        }
    }

    /// A unique temporary directory for a single test, removed on drop. Mirrors
    /// the helpers used by the vault/session tests to avoid an extra dev-dep.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir =
                std::env::temp_dir().join(format!("keyhaven-cmd-test-{pid}-{n}-{nanos}"));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir { path: dir }
        }

        fn vault_path(&self) -> PathBuf {
            self.path.join("test.khv")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn create_with_recovery_returns_key_once_and_unlocks() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();

        let result = create_vault_impl(
            &session,
            "correct horse battery staple".to_string(),
            true,
            &path,
            fast_params(),
        )
        .expect("create with recovery");

        // Recovery key is returned exactly once (Req 2.2).
        let recovery_key = result
            .recovery_key
            .expect("recovery key must be present when requested");
        assert!(!recovery_key.is_empty(), "recovery key must be non-empty");

        // The vault is left unlocked in the session (Req 1.7).
        assert!(session.is_unlocked(), "vault must be unlocked after creation");
        assert!(path.exists(), "vault file must be written to disk");

        // The returned recovery key really opens the vault via the recovery path
        // (confirms the recovery wrap was stored — Req 2.2/2.3).
        let via_recovery =
            crate::vault::unlock_with_recovery_key(&path, recovery_key.as_bytes())
                .expect("recovery key must unlock the vault");
        assert!(via_recovery.has_recovery());
    }

    #[test]
    fn create_without_recovery_is_password_only() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();

        let result = create_vault_impl(
            &session,
            "master-password".to_string(),
            false,
            &path,
            fast_params(),
        )
        .expect("create password-only");

        // No recovery key is produced when recovery is declined (Req 2.6).
        assert!(
            result.recovery_key.is_none(),
            "no recovery key when recovery is declined"
        );
        assert!(session.is_unlocked(), "vault must be unlocked after creation");

        // The on-disk vault opens with the password and has no recovery section.
        let opened = unlock_with_password(&path, b"master-password").expect("unlock");
        assert!(
            !opened.has_recovery(),
            "password-only vault must have no recovery wrap"
        );

        // A recovery-unlock attempt on a password-only vault is rejected.
        let err = crate::vault::unlock_with_recovery_key(&path, b"anything").unwrap_err();
        assert!(matches!(
            err,
            crate::vault::VaultRepoError::NoRecoverySection
        ));
    }

    #[test]
    fn created_vault_opens_in_unlocked_state_with_empty_model() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();

        create_vault_impl(&session, "pw".to_string(), false, &path, fast_params())
            .expect("create");

        // Unlocked session exposes the (empty) decrypted model (Req 1.7).
        let entry_count = session
            .with_vault(|v| v.model().entries.len())
            .expect("session must be unlocked and accessible");
        assert_eq!(entry_count, 0, "a freshly created vault has no entries");
    }

    #[test]
    fn empty_password_is_rejected() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();

        let err = create_vault_impl(&session, String::new(), false, &path, fast_params())
            .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
        // A rejected creation must not leave a dangling unlocked session.
        assert!(!session.is_unlocked());
        assert!(!path.exists(), "no vault file written on invalid input");
    }

    #[test]
    fn empty_path_is_rejected() {
        let session = SessionManager::new();
        let err = create_vault_impl(
            &session,
            "pw".to_string(),
            false,
            Path::new(""),
            fast_params(),
        )
        .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
        assert!(!session.is_unlocked());
    }

    // ---- Task 5.2: unlock commands ----

    /// Create a vault on disk (in its own session), then return the path so a
    /// fresh session can exercise the unlock commands as if the app restarted.
    fn create_on_disk(
        password: &str,
        generate_recovery: bool,
    ) -> (TempDir, PathBuf, Option<String>) {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        let result = create_vault_impl(
            &session,
            password.to_string(),
            generate_recovery,
            &path,
            fast_params(),
        )
        .expect("create on disk");
        (tmp, path, result.recovery_key)
    }

    #[test]
    fn unlock_with_password_correct_and_wrong() {
        let (_tmp, path, _) = create_on_disk("pw-correct", false);

        // Wrong password is rejected and leaves the session locked.
        let session = SessionManager::new();
        let err = unlock_with_password_impl(&session, "pw-wrong".to_string(), &path).unwrap_err();
        assert!(matches!(err, KeyhavenError::WrongCredentials), "got {err:?}");
        assert!(!session.is_unlocked(), "failed unlock must not open the session");

        // Correct password unlocks and yields a non-recovery summary.
        let summary = unlock_with_password_impl(&session, "pw-correct".to_string(), &path)
            .expect("correct password unlocks");
        assert!(!summary.has_recovery);
        assert!(!summary.unlocked_via_recovery);
        assert_eq!(summary.entry_count, 0);
        assert!(session.is_unlocked());
    }

    #[test]
    fn unlock_with_password_rejects_empty_inputs() {
        let (_tmp, path, _) = create_on_disk("pw", false);
        let session = SessionManager::new();

        let err = unlock_with_password_impl(&session, String::new(), &path).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));

        let err = unlock_with_password_impl(&session, "pw".to_string(), Path::new("")).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn unlock_with_recovery_key_unlocks_and_flags_recovery() {
        let (_tmp, path, recovery) = create_on_disk("pw", true);
        let recovery_key = recovery.expect("recovery key");

        let session = SessionManager::new();
        let summary = unlock_with_recovery_key_impl(&session, recovery_key, &path)
            .expect("recovery key unlocks");
        assert!(summary.has_recovery);
        assert!(
            summary.unlocked_via_recovery,
            "recovery unlock must flag so the UI prompts for a new master password"
        );
        assert!(session.is_unlocked());
    }

    #[test]
    fn unlock_with_recovery_key_wrong_is_rejected() {
        let (_tmp, path, _recovery) = create_on_disk("pw", true);
        let session = SessionManager::new();

        let err =
            unlock_with_recovery_key_impl(&session, "WRONG-0000-0000".to_string(), &path)
                .unwrap_err();
        assert!(matches!(err, KeyhavenError::WrongCredentials), "got {err:?}");
        assert!(!session.is_unlocked());
    }

    #[test]
    fn unlock_with_recovery_key_on_password_only_vault_is_wrong_credentials() {
        // A vault with no recovery section: a recovery-key unlock surfaces as a
        // credentials failure to the user (Req 3.3).
        let (_tmp, path, _) = create_on_disk("pw", false);
        let session = SessionManager::new();

        let err =
            unlock_with_recovery_key_impl(&session, "ANYTHING-1234".to_string(), &path).unwrap_err();
        assert!(matches!(err, KeyhavenError::WrongCredentials), "got {err:?}");
    }

    // ---- Task 5.2: change master password ----

    #[test]
    fn change_master_password_updates_disk_and_keeps_recovery() {
        // Created unlocked in this session with a recovery key.
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        let recovery_key = create_vault_impl(
            &session,
            "old-pw".to_string(),
            true,
            &path,
            fast_params(),
        )
        .expect("create")
        .recovery_key
        .expect("recovery key");

        change_master_password_impl(&session, "old-pw".to_string(), "new-pw".to_string())
            .expect("change with correct current password");

        // Reopen from disk: old rejected, new accepted, recovery intact.
        assert!(matches!(
            unlock_with_password(&path, b"old-pw").unwrap_err(),
            crate::vault::VaultRepoError::WrongCredentials
        ));
        unlock_with_password(&path, b"new-pw").expect("new password unlocks from disk");
        crate::vault::unlock_with_recovery_key(&path, recovery_key.as_bytes())
            .expect("recovery key still unlocks after change");
    }

    #[test]
    fn change_master_password_wrong_current_is_rejected() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        create_vault_impl(&session, "old-pw".to_string(), false, &path, fast_params())
            .expect("create");

        let err =
            change_master_password_impl(&session, "wrong".to_string(), "new-pw".to_string())
                .unwrap_err();
        assert!(matches!(err, KeyhavenError::WrongCredentials), "got {err:?}");

        // The original password still opens the vault from disk.
        unlock_with_password(&path, b"old-pw").expect("old password still valid after rejection");
    }

    #[test]
    fn change_master_password_when_locked_is_locked_error() {
        let session = SessionManager::new();
        let err =
            change_master_password_impl(&session, "a".to_string(), "b".to_string()).unwrap_err();
        assert!(matches!(err, KeyhavenError::Locked), "got {err:?}");
    }

    #[test]
    fn change_master_password_rejects_empty_inputs() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        create_vault_impl(&session, "pw".to_string(), false, &path, fast_params())
            .expect("create");

        let err =
            change_master_password_impl(&session, String::new(), "new".to_string()).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));

        let err =
            change_master_password_impl(&session, "pw".to_string(), String::new()).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    /// Recovery flow end to end (Req 2.7): unlock with the recovery key, then set
    /// a new master password (authorizing with the recovery key), and confirm the
    /// new password works and recovery still works afterward.
    #[test]
    fn recovery_unlock_then_change_password_via_session() {
        let (_tmp, path, recovery) = create_on_disk("forgotten", true);
        let recovery_key = recovery.expect("recovery key");

        let session = SessionManager::new();
        let summary =
            unlock_with_recovery_key_impl(&session, recovery_key.clone(), &path).expect("recovery unlock");
        assert!(summary.unlocked_via_recovery);

        // Set a new master password using the recovery key as the current secret.
        change_master_password_impl(&session, recovery_key.clone(), "fresh-pw".to_string())
            .expect("recovery flow permits setting a new master password");

        // New password unlocks; recovery key still unlocks; old password does not.
        unlock_with_password(&path, b"fresh-pw").expect("new password unlocks");
        crate::vault::unlock_with_recovery_key(&path, recovery_key.as_bytes())
            .expect("recovery still works after setting a new password");
        assert!(matches!(
            unlock_with_password(&path, b"forgotten").unwrap_err(),
            crate::vault::VaultRepoError::WrongCredentials
        ));
    }

    // ---- Task 6.1: entry CRUD commands (gating + atomic persistence) ----

    use crate::entries::{EntryInput, FieldInput};
    use crate::model::{FieldType, ServiceRef};

    fn entry_input(title: Option<&str>, fields: Vec<FieldInput>) -> EntryInput {
        EntryInput {
            service_ref: ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            title: title.map(|t| t.to_string()),
            fields,
        }
    }

    fn pw_field(label: &str, value: &str) -> FieldInput {
        FieldInput {
            id: None,
            label: label.to_string(),
            field_type: FieldType::Password,
            value: value.to_string(),
            secret: true,
        }
    }

    /// Create an unlocked vault on disk in its own session and return the session
    /// + path so tests can exercise the entry commands against real persistence.
    fn unlocked_on_disk() -> (TempDir, PathBuf, SessionManager) {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        create_vault_impl(&session, "pw".to_string(), false, &path, fast_params())
            .expect("create");
        (tmp, path, session)
    }

    #[test]
    fn entry_commands_are_locked_when_vault_is_locked() {
        let session = SessionManager::new();

        assert!(matches!(
            create_entry_impl(&session, entry_input(Some("x"), vec![])).unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            get_entry_impl(&session, "id").unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            update_entry_impl(&session, "id", entry_input(None, vec![])).unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            delete_entry_impl(&session, "id").unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            list_entries_impl(&session, None, None).unwrap_err(),
            KeyhavenError::Locked
        ));
    }

    #[test]
    fn create_entry_persists_through_atomic_write_path() {
        let (_tmp, path, session) = unlocked_on_disk();

        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");

        assert!(!saved.empty_warning, "a filled entry must not warn");
        let id = saved.entry.id.clone();

        // Reopen from disk in a fresh session: the entry was persisted.
        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(reopened.model().entries.len(), 1);
        assert_eq!(reopened.model().entries[0].id, id);
        assert_eq!(reopened.model().entries[0].fields[0].value, "hunter2");
    }

    #[test]
    fn create_entry_flags_empty_entry_without_blocking() {
        let (_tmp, path, session) = unlocked_on_disk();

        // No title, blank field value → empty warning, but still saved (Req 5.6).
        let saved = create_entry_impl(
            &session,
            entry_input(None, vec![pw_field("Password", "")]),
        )
        .expect("create empty entry");
        assert!(saved.empty_warning, "blank entry must raise the warning flag");

        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(
            reopened.model().entries.len(),
            1,
            "the empty entry is still persisted (warn, not block)"
        );
    }

    #[test]
    fn update_entry_bumps_updated_at_and_persists() {
        let (_tmp, path, session) = unlocked_on_disk();

        let created = create_entry_impl(
            &session,
            entry_input(Some("Original"), vec![pw_field("Password", "old")]),
        )
        .expect("create")
        .entry;

        let updated = update_entry_impl(
            &session,
            &created.id,
            entry_input(Some("Edited"), vec![pw_field("Password", "new")]),
        )
        .expect("update")
        .entry;

        // Identity + created_at preserved; updated_at refreshed (Req 8.5).
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, created.created_at);
        assert!(
            updated.updated_at >= created.updated_at,
            "updated_at must not move backwards"
        );
        assert_eq!(updated.title.as_deref(), Some("Edited"));

        // Persisted: reopening from disk shows the edit.
        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(reopened.model().entries[0].title.as_deref(), Some("Edited"));
        assert_eq!(reopened.model().entries[0].fields[0].value, "new");
    }

    #[test]
    fn update_unknown_entry_is_invalid_input() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let err = update_entry_impl(&session, "no-such-id", entry_input(None, vec![]))
            .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn get_entry_returns_stored_entry() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let created = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "v")]),
        )
        .expect("create")
        .entry;

        let fetched = get_entry_impl(&session, &created.id).expect("get");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.title.as_deref(), Some("Personal"));

        assert!(matches!(
            get_entry_impl(&session, "no-such-id").unwrap_err(),
            KeyhavenError::InvalidInput { .. }
        ));
    }

    #[test]
    fn delete_entry_removes_and_persists() {
        let (_tmp, path, session) = unlocked_on_disk();
        let a = create_entry_impl(&session, entry_input(Some("A"), vec![pw_field("P", "1")]))
            .expect("create a")
            .entry;
        let b = create_entry_impl(&session, entry_input(Some("B"), vec![pw_field("P", "2")]))
            .expect("create b")
            .entry;

        delete_entry_impl(&session, &a.id).expect("delete a");

        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(reopened.model().entries.len(), 1);
        assert_eq!(reopened.model().entries[0].id, b.id);

        // Deleting a missing id is an invalid-input error.
        assert!(matches!(
            delete_entry_impl(&session, &a.id).unwrap_err(),
            KeyhavenError::InvalidInput { .. }
        ));
    }

    #[test]
    fn list_entries_returns_filtered_paginated_summaries_when_unlocked() {
        let (_tmp, _path, session) = unlocked_on_disk();
        create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "secretpw")]),
        )
        .expect("create a");
        create_entry_impl(
            &session,
            entry_input(Some("Work"), vec![pw_field("Password", "otherpw")]),
        )
        .expect("create b");

        // No query lists both as summaries.
        let all = list_entries_impl(&session, None, None).expect("list all");
        assert_eq!(all.total, 2);
        assert_eq!(all.entries.len(), 2);

        // Filter by title narrows results.
        let filtered = list_entries_impl(&session, Some("Personal".to_string()), None)
            .expect("list filtered");
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.entries[0].title.as_deref(), Some("Personal"));

        // Secret values are never matched, even when unlocked.
        let secret_search =
            list_entries_impl(&session, Some("secretpw".to_string()), None).expect("list");
        assert_eq!(secret_search.total, 0, "secret values must not be searchable");
    }

    // ---- Task 7.2: catalog search & custom services ----

    use crate::model::IconRef;

    #[test]
    fn search_catalog_command_matches_by_name_and_alias() {
        // By name (case-insensitive).
        let by_name = search_catalog_impl("google").expect("search");
        assert!(by_name.iter().any(|s| s.id == "google"));

        // By alias.
        let by_alias = search_catalog_impl("gmail").expect("search");
        assert!(by_alias.iter().any(|s| s.id == "google"));

        // No match → empty.
        let none = search_catalog_impl("zzz-not-a-service-zzz").expect("search");
        assert!(none.is_empty());
    }

    #[test]
    fn search_catalog_needs_no_unlocked_vault() {
        // The catalog is static, non-secret data: searching it requires no
        // session and never returns `Locked`.
        let all = search_catalog_impl("").expect("search with no session");
        assert!(all.len() >= 50, "empty query returns the whole catalog");
    }

    #[test]
    fn create_custom_service_persists_and_lists() {
        let (_tmp, path, session) = unlocked_on_disk();

        let created = create_custom_service_impl(
            &session,
            "My Home Server".to_string(),
            IconRef::Data {
                reference: "data:image/png;base64,AAAA".to_string(),
            },
        )
        .expect("create custom service");

        // A fresh UUID id is minted, and the custom indicator flag is set.
        assert!(
            uuid::Uuid::parse_str(&created.id).is_ok(),
            "custom service id must be a UUID"
        );
        assert!(created.custom, "result must carry the custom indicator flag");
        assert_eq!(created.name, "My Home Server");

        // It is listed by the list command.
        let listed = list_custom_services_impl(&session).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert!(listed[0].custom);

        // It is persisted: reopen the vault from disk and confirm it is present
        // with its icon (IconRef) intact.
        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(reopened.model().custom_services.len(), 1);
        let persisted = &reopened.model().custom_services[0];
        assert_eq!(persisted.id, created.id);
        assert_eq!(persisted.name, "My Home Server");
        assert_eq!(
            persisted.icon,
            IconRef::Data {
                reference: "data:image/png;base64,AAAA".to_string(),
            },
            "custom icon must round-trip through persistence"
        );
    }

    #[test]
    fn create_custom_service_with_builtin_icon_round_trips_via_serialization() {
        let (_tmp, _path, session) = unlocked_on_disk();

        let created = create_custom_service_impl(
            &session,
            "Internal Tool".to_string(),
            IconRef::Builtin {
                reference: "tool.svg".to_string(),
            },
        )
        .expect("create");

        // The wire view serializes with a camelCase `custom: true` flag and an
        // icon that serializes to its tagged shape intact.
        let json = serde_json::to_value(&created).expect("serialize view");
        assert_eq!(json["custom"], true);
        assert_eq!(json["name"], "Internal Tool");
        assert_eq!(json["icon"]["kind"], "builtin");
        assert_eq!(json["icon"]["ref"], "tool.svg");
        assert_eq!(json["id"], created.id);
    }

    #[test]
    fn create_custom_service_when_locked_returns_locked() {
        let session = SessionManager::new();
        let err = create_custom_service_impl(
            &session,
            "Anything".to_string(),
            IconRef::Builtin {
                reference: "x.svg".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, KeyhavenError::Locked), "got {err:?}");
    }

    #[test]
    fn create_custom_service_empty_name_is_invalid_input() {
        let (_tmp, _path, session) = unlocked_on_disk();

        // Blank/whitespace-only names are rejected before touching the vault.
        for name in ["", "   "] {
            let err = create_custom_service_impl(
                &session,
                name.to_string(),
                IconRef::Builtin {
                    reference: "x.svg".to_string(),
                },
            )
            .unwrap_err();
            assert!(matches!(err, KeyhavenError::InvalidInput { .. }), "got {err:?}");
        }

        // Nothing was persisted by the rejected calls.
        assert_eq!(
            list_custom_services_impl(&session).expect("list").len(),
            0
        );
    }

    #[test]
    fn list_custom_services_when_locked_returns_locked() {
        let session = SessionManager::new();
        let err = list_custom_services_impl(&session).unwrap_err();
        assert!(matches!(err, KeyhavenError::Locked), "got {err:?}");
    }

    #[test]
    fn multiple_custom_services_get_distinct_ids() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let a = create_custom_service_impl(
            &session,
            "Service A".to_string(),
            IconRef::Builtin {
                reference: "a.svg".to_string(),
            },
        )
        .expect("create a");
        let b = create_custom_service_impl(
            &session,
            "Service B".to_string(),
            IconRef::Builtin {
                reference: "b.svg".to_string(),
            },
        )
        .expect("create b");

        assert_ne!(a.id, b.id, "each custom service gets a unique id");
        assert_eq!(list_custom_services_impl(&session).expect("list").len(), 2);
    }

    // ---- Task 8.2: clipboard copy with auto-clear ----

    /// In-memory fake clipboard so the copy/schedule wiring can be exercised
    /// without a Tauri runtime. Records the last written text and any scheduled
    /// clear request (value + delay) for assertions.
    struct FakeClipboard {
        written: std::cell::RefCell<Option<String>>,
        scheduled: std::cell::RefCell<Option<(String, u32)>>,
    }

    impl FakeClipboard {
        fn new() -> Self {
            FakeClipboard {
                written: std::cell::RefCell::new(None),
                scheduled: std::cell::RefCell::new(None),
            }
        }
    }

    impl ClipboardSink for FakeClipboard {
        fn write_text(&self, text: &str) -> Result<(), String> {
            *self.written.borrow_mut() = Some(text.to_string());
            Ok(())
        }

        fn schedule_clear(&self, copied: String, clear_seconds: u32) {
            *self.scheduled.borrow_mut() = Some((copied, clear_seconds));
        }
    }

    /// Req 8.4 / clear-if-unchanged: only an unchanged clipboard is cleared.
    #[test]
    fn should_clear_only_when_clipboard_is_unchanged() {
        // Still holds exactly what we copied → clear it.
        assert!(should_clear_clipboard(Some("secret-value"), "secret-value"));
        // User copied something else since → leave it alone.
        assert!(!should_clear_clipboard(
            Some("user copied this later"),
            "secret-value"
        ));
        // Clipboard unreadable / non-text → don't touch it.
        assert!(!should_clear_clipboard(None, "secret-value"));
        // Subtle difference (trailing space) is still a change.
        assert!(!should_clear_clipboard(Some("secret-value "), "secret-value"));
    }

    /// `lookup_field_value` returns the value plus the configured clear delay.
    #[test]
    fn lookup_field_value_returns_value_and_clear_delay() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");
        let entry_id = saved.entry.id.clone();
        let field_id = saved.entry.fields[0].id.clone();

        let (value, clear_seconds) = session
            .with_vault(|v| lookup_field_value(v.model(), &entry_id, &field_id))
            .expect("unlocked")
            .expect("field found");

        assert_eq!(value, "hunter2");
        // Default clipboard clear delay from Settings::default().
        assert_eq!(clear_seconds, 20);
    }

    #[test]
    fn lookup_field_value_rejects_unknown_entry_and_field() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");
        let entry_id = saved.entry.id.clone();

        // Unknown entry id.
        let err = session
            .with_vault(|v| lookup_field_value(v.model(), "no-such-entry", "f"))
            .expect("unlocked")
            .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));

        // Known entry, unknown field id.
        let err = session
            .with_vault(|v| lookup_field_value(v.model(), &entry_id, "no-such-field"))
            .expect("unlocked")
            .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn copy_secret_is_gated_when_locked() {
        let session = SessionManager::new();
        let clipboard = FakeClipboard::new();

        let err =
            copy_secret_to_clipboard_impl(&clipboard, &session, "e", "f").unwrap_err();
        assert!(matches!(err, KeyhavenError::Locked));
        // Nothing was written to the clipboard while locked.
        assert!(clipboard.written.borrow().is_none());
    }

    #[test]
    fn copy_secret_writes_value_and_schedules_clear() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");
        let entry_id = saved.entry.id.clone();
        let field_id = saved.entry.fields[0].id.clone();

        let clipboard = FakeClipboard::new();
        copy_secret_to_clipboard_impl(&clipboard, &session, &entry_id, &field_id)
            .expect("copy");

        // The secret value is on the clipboard...
        assert_eq!(clipboard.written.borrow().as_deref(), Some("hunter2"));
        // ...and a clear-if-unchanged task is armed with the copied value and the
        // configured 20s delay (Req 8.3, 8.4).
        assert_eq!(
            *clipboard.scheduled.borrow(),
            Some(("hunter2".to_string(), 20))
        );
    }

    #[test]
    fn copy_secret_with_zero_delay_does_not_schedule_clear() {
        let (_tmp, _path, session) = unlocked_on_disk();
        // Disable auto-clear (0 = never clear).
        session
            .with_vault_mut(|v| v.model_mut().settings.clipboard_clear_seconds = 0)
            .expect("unlocked");
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");
        let entry_id = saved.entry.id.clone();
        let field_id = saved.entry.fields[0].id.clone();

        let clipboard = FakeClipboard::new();
        copy_secret_to_clipboard_impl(&clipboard, &session, &entry_id, &field_id)
            .expect("copy");

        // Value is still copied, but no auto-clear is scheduled.
        assert_eq!(clipboard.written.borrow().as_deref(), Some("hunter2"));
        assert!(clipboard.scheduled.borrow().is_none());
    }

    #[test]
    fn copy_secret_rejects_unknown_field() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");
        let entry_id = saved.entry.id.clone();

        let clipboard = FakeClipboard::new();
        let err =
            copy_secret_to_clipboard_impl(&clipboard, &session, &entry_id, "no-such-field")
                .unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
        assert!(clipboard.written.borrow().is_none());
    }

    // ---- Task 8.3: settings persistence and vault export/import ----

    use crate::model::Settings;

    /// Whether `haystack` contains the contiguous byte sequence `needle`. Used to
    /// assert that secret plaintext never appears in an exported (encrypted) file.
    fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
        needle.len() <= haystack.len()
            && haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Write `bytes` to a file named `name` inside `dir`, returning its path.
    fn write_temp_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).expect("write temp file");
        p
    }

    #[test]
    fn get_settings_reflects_defaults_and_is_locked_when_locked() {
        // Defaults are returned from a freshly created vault.
        let (_tmp, _path, session) = unlocked_on_disk();
        assert_eq!(
            get_settings_impl(&session).expect("get settings"),
            Settings::default()
        );

        // Gated when locked (settings live inside the encrypted vault).
        let locked = SessionManager::new();
        assert!(matches!(
            get_settings_impl(&locked).unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            update_settings_impl(&locked, Settings::default()).unwrap_err(),
            KeyhavenError::Locked
        ));
    }

    #[test]
    fn update_settings_round_trips_and_persists() {
        let (_tmp, path, session) = unlocked_on_disk();

        let mut updated = Settings::default();
        updated.auto_lock_seconds = 30;
        updated.lock_on_blur = true;
        updated.clipboard_clear_seconds = 5;
        updated.password_gen_defaults.length = 32;
        updated.password_gen_defaults.symbols = false;

        update_settings_impl(&session, updated.clone()).expect("update settings");

        // Reflected immediately in the live session.
        assert_eq!(get_settings_impl(&session).expect("get"), updated);

        // Persisted through the atomic write path: reopen from disk and confirm.
        let reopened = unlock_with_password(&path, b"pw").expect("reopen");
        assert_eq!(reopened.model().settings, updated);
    }

    #[test]
    fn export_produces_a_valid_openable_encrypted_file() {
        let (tmp, _path, session) = unlocked_on_disk();
        // Add an entry so the export carries real (secret) content.
        create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry");

        let dest = tmp.path.join("export.khv");
        export_vault_impl(&session, &dest).expect("export");

        assert!(dest.exists(), "export file must be written");

        let bytes = std::fs::read(&dest).expect("read export");
        // It remains a Keyhaven vault file (begins with MAGIC)...
        assert_eq!(&bytes[..crate::vault::MAGIC.len()], crate::vault::MAGIC);
        // ...and stays encrypted: the secret value is not present in plaintext
        // anywhere in the exported bytes (Req 11.2; Property 2).
        assert!(
            !contains_subslice(&bytes, b"hunter2"),
            "exported file must not contain the secret in plaintext"
        );

        // It opens with the same master password and the data matches (Req 11.2).
        let opened = unlock_with_password(&dest, b"pw").expect("export opens with master password");
        assert_eq!(opened.model().entries.len(), 1);
        assert_eq!(opened.model().entries[0].fields[0].value, "hunter2");
    }

    #[test]
    fn export_remains_openable_with_recovery_key() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        let recovery = create_vault_impl(&session, "pw".to_string(), true, &path, fast_params())
            .expect("create")
            .recovery_key
            .expect("recovery key");

        let dest = tmp.path.join("backup.khv");
        export_vault_impl(&session, &dest).expect("export");

        // The backup opens with either secret (Req 11.2).
        unlock_with_password(&dest, b"pw").expect("export opens with master password");
        crate::vault::unlock_with_recovery_key(&dest, recovery.as_bytes())
            .expect("export opens with recovery key");
    }

    #[test]
    fn export_is_locked_when_vault_is_locked() {
        let session = SessionManager::new();
        let tmp = TempDir::new();
        let dest = tmp.path.join("export.khv");

        let err = export_vault_impl(&session, &dest).unwrap_err();
        assert!(matches!(err, KeyhavenError::Locked), "got {err:?}");
        assert!(!dest.exists(), "nothing must be exported while locked");
    }

    #[test]
    fn export_rejects_empty_and_self_destination() {
        let (_tmp, path, session) = unlocked_on_disk();

        // Empty destination.
        let err = export_vault_impl(&session, Path::new("")).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }), "got {err:?}");

        // Exporting onto the live vault file itself is refused (would corrupt it).
        let err = export_vault_impl(&session, &path).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }), "got {err:?}");
        // The source vault is untouched and still opens.
        unlock_with_password(&path, b"pw").expect("source vault still opens");
    }

    #[test]
    fn import_rejects_foreign_file_without_altering_current_vault() {
        let (tmp, path, _session) = unlocked_on_disk();
        let before = std::fs::read(&path).expect("read current vault");

        // A file that is not a Keyhaven vault (bad magic).
        let foreign = write_temp_file(&tmp.path, "foreign.bin", b"this is not a keyhaven vault");
        let err = import_vault_impl(&foreign).unwrap_err();
        assert!(matches!(err, KeyhavenError::VaultCorrupted), "got {err:?}");

        // The existing vault is byte-for-byte unchanged (Property 9) and still opens.
        assert_eq!(
            before,
            std::fs::read(&path).expect("read current vault"),
            "a failed import must not alter the existing vault"
        );
        unlock_with_password(&path, b"pw").expect("existing vault still unlocks");
    }

    #[test]
    fn import_rejects_incompatible_version_without_altering_current_vault() {
        let (tmp, path, _session) = unlocked_on_disk();
        let before = std::fs::read(&path).expect("read current vault");

        // Build a structurally real vault, then bump FORMAT_VER beyond supported.
        let other = TempDir::new();
        let other_path = other.vault_path();
        let s2 = SessionManager::new();
        create_vault_impl(&s2, "pw2".to_string(), false, &other_path, fast_params())
            .expect("create other vault");
        let mut bytes = std::fs::read(&other_path).expect("read other vault");
        let newer = crate::vault::FORMAT_VERSION + 1;
        let magic_len = crate::vault::MAGIC.len();
        bytes[magic_len..magic_len + 2].copy_from_slice(&newer.to_le_bytes());
        let bad = write_temp_file(&tmp.path, "newer.khv", &bytes);

        let err = import_vault_impl(&bad).unwrap_err();
        assert!(matches!(err, KeyhavenError::IncompatibleVersion), "got {err:?}");

        // The existing vault is unaltered (Req 11.6; Property 9).
        assert_eq!(
            before,
            std::fs::read(&path).expect("read current vault"),
            "a failed import must not alter the existing vault"
        );
    }

    #[test]
    fn import_accepts_a_valid_vault_file_and_leaves_current_vault_untouched() {
        let (_tmp, path, _session) = unlocked_on_disk();
        let before = std::fs::read(&path).expect("read current vault");

        // A separate, valid Keyhaven vault is a legitimate import source.
        let other = TempDir::new();
        let other_path = other.vault_path();
        let s2 = SessionManager::new();
        create_vault_impl(&s2, "pw2".to_string(), false, &other_path, fast_params())
            .expect("create other vault");

        import_vault_impl(&other_path).expect("a valid vault file imports successfully");

        // Validation performs no writes, so the current vault is unchanged.
        assert_eq!(
            before,
            std::fs::read(&path).expect("read current vault"),
            "a successful import (validation) must not alter the existing vault"
        );
    }

    #[test]
    fn import_rejects_empty_and_missing_source() {
        // Empty source path.
        let err = import_vault_impl(Path::new("")).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }), "got {err:?}");

        // Non-existent file surfaces as an I/O error (read failure) — not a
        // corruption, and crucially still no write to any existing vault.
        let missing = std::env::temp_dir().join("keyhaven-import-does-not-exist-xyz.khv");
        let _ = std::fs::remove_file(&missing);
        let err = import_vault_impl(&missing).unwrap_err();
        assert!(matches!(err, KeyhavenError::Io { .. }), "got {err:?}");
    }

    // =======================================================================
    // Task 15 — Tauri integration smoke tests
    // =======================================================================
    //
    // These exercise the **full vault lifecycle through the command-handler
    // implementations** (`*_impl`) end to end against a temporary vault file:
    //
    //   create_vault → unlock → entry CRUD (create/read/update/delete) → lock
    //
    // They are deliberately "smoke" tests: rather than re-proving the fine-
    // grained behavior already covered above, they confirm the command layer
    // composes correctly as a whole — the session gate, the atomic write path,
    // disk persistence across a simulated app restart, and the lock/zeroize +
    // `vault-locked` notification all working together. Everything runs against
    // a `TempDir` that is removed on drop, so no artifacts persist (Req 13.x).

    /// A test [`LockNotifier`] that counts how many times the vault locked, so
    /// the smoke test can assert the `vault-locked` notification fired without a
    /// Tauri runtime.
    struct RecordingNotifier {
        count: std::cell::Cell<usize>,
    }

    impl RecordingNotifier {
        fn new() -> Self {
            RecordingNotifier {
                count: std::cell::Cell::new(0),
            }
        }
    }

    impl crate::session::LockNotifier for RecordingNotifier {
        fn notify_locked(&self) {
            self.count.set(self.count.get() + 1);
        }
    }

    /// Full lifecycle: create (with recovery) → CRUD → lock → confirm gated →
    /// reopen from disk → confirm persistence → final lock. All against a temp
    /// vault that is cleaned up on drop.
    #[test]
    fn smoke_full_lifecycle_create_unlock_crud_lock() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let session = SessionManager::new();
        let notifier = RecordingNotifier::new();

        // --- create_vault → opens unlocked (Req 1.7), recovery key returned once ---
        let recovery = create_vault_impl(
            &session,
            "correct horse battery staple".to_string(),
            true,
            &path,
            fast_params(),
        )
        .expect("create vault")
        .recovery_key
        .expect("recovery key returned exactly once");
        assert!(session.is_unlocked(), "vault opens unlocked after creation");
        assert!(path.exists(), "vault file written to disk");

        // --- entry CRUD against the unlocked session ---
        // Create.
        let created = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry")
        .entry;
        let entry_id = created.id.clone();

        // Read.
        let fetched = get_entry_impl(&session, &entry_id).expect("get entry");
        assert_eq!(fetched.fields[0].value, "hunter2");

        // Update.
        update_entry_impl(
            &session,
            &entry_id,
            entry_input(Some("Personal (edited)"), vec![pw_field("Password", "rotated-pw")]),
        )
        .expect("update entry");

        // List reflects exactly one entry; the secret value is never searchable.
        let listing = list_entries_impl(&session, None, None).expect("list");
        assert_eq!(listing.total, 1);
        assert_eq!(
            list_entries_impl(&session, Some("rotated-pw".to_string()), None)
                .expect("secret search")
                .total,
            0,
            "secret values must never be searchable"
        );

        // --- lock → zeroize + notify, then entry access is gated (Req 3.4/3.5) ---
        assert!(session.lock(&notifier), "lock reports it closed an open vault");
        assert!(!session.is_unlocked());
        assert_eq!(notifier.count.get(), 1, "vault-locked fired exactly once");
        assert!(matches!(
            get_entry_impl(&session, &entry_id).unwrap_err(),
            KeyhavenError::Locked
        ));
        assert!(matches!(
            list_entries_impl(&session, None, None).unwrap_err(),
            KeyhavenError::Locked
        ));

        // --- reopen from disk in a fresh session (simulated app restart) ---
        let fresh = SessionManager::new();
        let summary = unlock_with_password_impl(&fresh, "correct horse battery staple".to_string(), &path)
            .expect("reopen with master password");
        assert!(summary.has_recovery);
        assert_eq!(summary.entry_count, 1, "the edited entry persisted to disk");

        // The update persisted across the lifecycle.
        let reopened = get_entry_impl(&fresh, &entry_id).expect("entry present after reopen");
        assert_eq!(reopened.title.as_deref(), Some("Personal (edited)"));
        assert_eq!(reopened.fields[0].value, "rotated-pw");

        // The recovery key still opens the on-disk vault (either-key independence).
        crate::vault::unlock_with_recovery_key(&path, recovery.as_bytes())
            .expect("recovery key still unlocks the persisted vault");

        // Delete the entry, then confirm the empty state persists.
        delete_entry_impl(&fresh, &entry_id).expect("delete entry");
        let after_delete = SessionManager::new();
        let summary = unlock_with_password_impl(&after_delete, "correct horse battery staple".to_string(), &path)
            .expect("reopen after delete");
        assert_eq!(summary.entry_count, 0, "delete persisted to disk");

        // Final lock leaves the session closed and gated.
        let notifier2 = RecordingNotifier::new();
        assert!(after_delete.lock(&notifier2));
        assert!(!after_delete.is_unlocked());
    }

    /// The clipboard copy-with-auto-clear path composes with the live session:
    /// copy a secret from a real unlocked vault and confirm the value is written
    /// and a clear-if-unchanged task is armed — exercised through the
    /// command-impl with an in-memory clipboard.
    #[test]
    fn smoke_copy_secret_through_session() {
        let (_tmp, _path, session) = unlocked_on_disk();
        let saved = create_entry_impl(
            &session,
            entry_input(Some("Personal"), vec![pw_field("Password", "hunter2")]),
        )
        .expect("create entry")
        .entry;

        let clipboard = FakeClipboard::new();
        copy_secret_to_clipboard_impl(
            &clipboard,
            &session,
            &saved.id,
            &saved.fields[0].id,
        )
        .expect("copy secret");

        assert_eq!(clipboard.written.borrow().as_deref(), Some("hunter2"));
        assert_eq!(
            *clipboard.scheduled.borrow(),
            Some(("hunter2".to_string(), 20))
        );
    }
}
