//! Vault repository: create/open flows and atomic, crash-safe disk writes.
//!
//! This module is the **I/O and orchestration layer** of the vault. It ties
//! together the three lower layers built in earlier tasks and adds durable,
//! crash-safe persistence:
//!
//! - the crypto core ([`crate::crypto`], task 2) — Argon2id derivation, AEAD,
//!   CSPRNG, and VEK wrapping/unwrapping ([`crate::crypto::envelope`]);
//! - the decrypted data model ([`crate::model`], task 3.1) — JSON + DEFLATE
//!   (de)serialization of the [`VaultModel`];
//! - the on-disk binary header ([`crate::vault::header`], task 3.2) — the
//!   secret-free [`VaultFile`] layout.
//!
//! It performs no cryptography of its own beyond calling those helpers; its job
//! is to sequence them correctly and move bytes safely to and from disk.
//!
//! ## Crypto flows (design "Vault Encryption Flow")
//!
//! **Create** ([`create_vault`]): generate a random VEK + `MASTER_SALT`, wrap
//! the VEK under the master password (`PW_WRAP`); optionally generate a
//! human-friendly recovery key + `REC_SALT` and wrap the VEK under it
//! (`REC_WRAP`); encrypt the compressed empty model under the VEK; write the
//! file atomically. The recovery key is returned **exactly once** (Req 2.2) — it
//! is never stored in any recoverable form (Req 1.8, 2.x).
//!
//! **Unlock (password)** ([`unlock_with_password`]) / **unlock (recovery)**
//! ([`unlock_with_recovery_key`]): decode the file, unwrap the VEK from the
//! appropriate wrap, decrypt + deserialize the payload, and return an
//! [`OpenVault`] holding the VEK (in [`Zeroizing`]), the decrypted model, and
//! the header context needed for later saves. A wrong secret surfaces as
//! [`VaultRepoError::WrongCredentials`] (Req 2.8, 3.3); a tampered/corrupt
//! payload surfaces as [`VaultRepoError::Corrupted`] (Req 3.6).
//!
//! **Save** ([`OpenVault::save`]): re-encrypt the current model under the VEK
//! with a **fresh** payload nonce, rebuild the [`VaultFile`] preserving all
//! header fields (salts, params, wraps), and write atomically.
//!
//! ## Atomic, crash-safe writes (Req 1.5, 1.7; Property 8)
//!
//! [`atomic_write`] writes to a uniquely-named temp file **in the same
//! directory** as the target, flushes it to stable storage with `sync_all`
//! (fsync), then — if a prior vault exists — copies it to a `.bak` sibling, and
//! finally renames the temp file over the target. `std::fs::rename` replaces an
//! existing destination atomically on both Unix and Windows, so a crash at any
//! point leaves either the previous complete vault (temp not yet renamed) or the
//! new complete vault, plus a recoverable `.bak` of the prior version — never a
//! half-written primary file.
//!
//! The prior vault is **copied** (not moved) to `.bak` so the primary remains
//! intact for the entire window before the final rename. Directory fsync is
//! performed best-effort on Unix; it is not reliably available on Windows, so it
//! is skipped there (the rename itself is still atomic).

// The repository API is consumed by the command layer (task 5); not all of it
// is referenced from the binary yet while the core is built bottom-up.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::crypto::envelope::{unwrap_vek_with_secret, wrap_vek_with_secret, EnvelopeError};
use crate::crypto::{
    aead_decrypt, aead_encrypt, fill_random, random_salt, random_vek, AeadCiphertext, CryptoError,
    KdfParams, VEK_LEN,
};
use crate::model::{deserialize_vault, serialize_vault, ModelError, VaultModel};
use crate::vault::header::{decode, VaultFile, VaultFormatError};

/// Number of random bytes behind a generated recovery key (160 bits of
/// entropy). Encoded as Crockford base32 this yields 32 symbols.
const RECOVERY_KEY_BYTES: usize = 20;

/// Crockford base32 alphabet (excludes I, L, O, U to avoid visual ambiguity),
/// used to render the recovery key in a human-friendly form.
const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Errors produced by the vault repository.
///
/// This composes the lower-layer error types and adds the distinctions the
/// command layer (task 5) needs to map onto the design's `KeyhavenError`:
/// - [`VaultRepoError::WrongCredentials`] → `KeyhavenError::WrongCredentials`
///   (an incorrect master password or recovery key).
/// - [`VaultRepoError::Corrupted`] / [`VaultRepoError::Format`] with
///   [`VaultFormatError::VaultCorrupted`] → `KeyhavenError::VaultCorrupted`.
/// - [`VaultRepoError::Format`] with [`VaultFormatError::IncompatibleVersion`]
///   → `KeyhavenError::IncompatibleVersion`.
/// - [`VaultRepoError::Io`] → `KeyhavenError::Io`.
/// - [`VaultRepoError::NoRecoverySection`] → recovery requested on a vault that
///   has none.
#[derive(Debug)]
pub enum VaultRepoError {
    /// The supplied master password or recovery key could not unwrap the VEK
    /// (AEAD auth-tag failure). Deliberately opaque — a wrong secret and a
    /// tampered wrap are indistinguishable (Req 2.8, 3.3; Property 4).
    WrongCredentials,
    /// A recovery-key unlock was attempted but the vault has no recovery section.
    NoRecoverySection,
    /// The decrypted payload failed authentication or could not be parsed: the
    /// file is damaged or has been tampered with (Req 3.6; Property 3). The VEK
    /// itself unwrapped correctly, distinguishing this from `WrongCredentials`.
    Corrupted,
    /// A vault-file format/version error from decoding the header.
    Format(VaultFormatError),
    /// An envelope (VEK wrap/unwrap) error that is not a credentials failure.
    Envelope(EnvelopeError),
    /// A model (de)serialization/compression error.
    Model(ModelError),
    /// A low-level cryptographic error (e.g. AEAD encryption while saving).
    Crypto(CryptoError),
    /// A filesystem I/O error while reading or writing the vault.
    Io(std::io::Error),
}

impl std::fmt::Display for VaultRepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultRepoError::WrongCredentials => {
                write!(f, "incorrect master password or recovery key")
            }
            VaultRepoError::NoRecoverySection => {
                write!(f, "this vault has no recovery key configured")
            }
            VaultRepoError::Corrupted => {
                write!(f, "this vault file is damaged or not a Keyhaven vault")
            }
            VaultRepoError::Format(e) => write!(f, "{e}"),
            VaultRepoError::Envelope(e) => write!(f, "{e}"),
            VaultRepoError::Model(e) => write!(f, "{e}"),
            VaultRepoError::Crypto(e) => write!(f, "{e}"),
            VaultRepoError::Io(e) => write!(f, "vault I/O error: {e}"),
        }
    }
}

impl std::error::Error for VaultRepoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VaultRepoError::Format(e) => Some(e),
            VaultRepoError::Envelope(e) => Some(e),
            VaultRepoError::Model(e) => Some(e),
            VaultRepoError::Crypto(e) => Some(e),
            VaultRepoError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<VaultFormatError> for VaultRepoError {
    fn from(e: VaultFormatError) -> Self {
        VaultRepoError::Format(e)
    }
}

impl From<EnvelopeError> for VaultRepoError {
    fn from(e: EnvelopeError) -> Self {
        // Keep WrongCredentials distinguishable; everything else stays distinct.
        match e {
            EnvelopeError::WrongCredentials => VaultRepoError::WrongCredentials,
            other => VaultRepoError::Envelope(other),
        }
    }
}

impl From<ModelError> for VaultRepoError {
    fn from(e: ModelError) -> Self {
        VaultRepoError::Model(e)
    }
}

impl From<CryptoError> for VaultRepoError {
    fn from(e: CryptoError) -> Self {
        VaultRepoError::Crypto(e)
    }
}

impl From<std::io::Error> for VaultRepoError {
    fn from(e: std::io::Error) -> Self {
        VaultRepoError::Io(e)
    }
}

/// An unlocked, in-memory vault handle.
///
/// Holds the Vault Encryption Key (in [`Zeroizing`], cleared on drop — Req 15.5),
/// the decrypted [`VaultModel`], and the [`VaultFile`] header context required to
/// persist subsequent changes without re-deriving keys. Returned by
/// [`create_vault`], [`unlock_with_password`], and [`unlock_with_recovery_key`].
///
/// Mutate the model via [`model_mut`](OpenVault::model_mut), then call
/// [`save`](OpenVault::save) to encrypt and write it back atomically.
pub struct OpenVault {
    /// The Vault Encryption Key recovered during create/unlock. Never persisted
    /// in the clear; only its wrapped form lives in the header.
    vek: Zeroizing<[u8; VEK_LEN]>,
    /// The header context (salts, KDF params, wraps, and the most recently
    /// written payload). [`save`](OpenVault::save) rewrites only the payload and
    /// its nonce, preserving every other field.
    file: VaultFile,
    /// The decrypted vault contents.
    model: VaultModel,
    /// The on-disk location this vault was created/opened from. Retained so that
    /// in-place rewrites (e.g. [`change_master_password`](OpenVault::change_master_password),
    /// which has no `path` parameter at the command layer) know where to write.
    /// Not secret.
    path: PathBuf,
}

impl std::fmt::Debug for OpenVault {
    /// Redacts the VEK and decrypted model so an [`OpenVault`] can be
    /// debug-printed (e.g. in test assertions) without ever leaking secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenVault")
            .field("vek", &"<redacted>")
            .field("model", &"<redacted>")
            .field("has_recovery", &self.file.recovery.is_some())
            .field("kdf_params", &self.file.kdf_params)
            .finish()
    }
}

impl OpenVault {
    /// Borrow the decrypted vault model.
    pub fn model(&self) -> &VaultModel {
        &self.model
    }

    /// Mutably borrow the decrypted vault model so callers (the entry repository
    /// in task 6) can apply changes before calling [`save`](OpenVault::save).
    pub fn model_mut(&mut self) -> &mut VaultModel {
        &mut self.model
    }

    /// Replace the decrypted model wholesale.
    pub fn set_model(&mut self, model: VaultModel) {
        self.model = model;
    }

    /// Borrow the recovered VEK. Exposed for the session manager (task 4); the
    /// key is held in [`Zeroizing`] and must never be persisted or logged.
    pub fn vek(&self) -> &[u8; VEK_LEN] {
        &self.vek
    }

    /// Whether this vault has a recovery-key section.
    pub fn has_recovery(&self) -> bool {
        self.file.recovery.is_some()
    }

    /// The KDF parameters recorded in the vault header.
    pub fn kdf_params(&self) -> KdfParams {
        self.file.kdf_params
    }

    /// The on-disk path this vault was created from or opened at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Change the master password in place, rewriting **only** `MASTER_SALT` and
    /// `PW_WRAP` while leaving the payload ciphertext and the recovery wrap
    /// untouched (design "Change master password"; Req 3.2, Property 6).
    ///
    /// `current_secret` must prove the caller can already open this vault: it is
    /// accepted when it unwraps the VEK from either the current password wrap
    /// (`PW_WRAP`) **or** — for the recovery flow (Req 2.7) — the recovery wrap
    /// (`REC_WRAP`). This covers both the Settings change-password path (the user
    /// supplies their current master password) and the recovery path (the user
    /// just unlocked with their recovery key and supplies it here to set a new
    /// master password). A secret matching neither wrap is rejected with
    /// [`VaultRepoError::WrongCredentials`] (Req 3.3) and nothing is written.
    ///
    /// The new wrap is derived under the vault's **existing** KDF params (only
    /// the salt is regenerated) so a later [`unlock_with_password`], which reads
    /// the params from the header, stays consistent. The VEK is unchanged, so
    /// the payload need not be — and is not — re-encrypted: the file is rewritten
    /// atomically with the original payload bytes preserved.
    pub fn change_master_password(
        &mut self,
        current_secret: &[u8],
        new_password: &[u8],
    ) -> Result<(), VaultRepoError> {
        // 1. Authorize: require a currently-valid secret (master password or, in
        //    the recovery flow, the recovery key). This never yields a usable VEK
        //    on failure (Property 4) — we only observe success/failure here.
        if !self.verify_secret(current_secret) {
            return Err(VaultRepoError::WrongCredentials);
        }

        // 2. Re-wrap the SAME VEK under the new password with a fresh salt,
        //    reusing the vault's existing KDF params.
        let new_salt = random_salt();
        let new_wrap =
            wrap_vek_with_secret(new_password, &new_salt, self.file.kdf_params, &self.vek)?;

        // 3. Rewrite ONLY MASTER_SALT + PW_WRAP; recovery wrap and payload (nonce
        //    + ciphertext) are left exactly as they were (Property 6).
        self.file.master_salt = new_salt;
        self.file.pw_wrap = new_wrap;

        atomic_write(&self.path, &self.file.encode())
    }

    /// Whether `secret` can currently open this vault via either the password
    /// wrap or, if present, the recovery wrap. Used to authorize an in-place
    /// master-password change. Both checks are deliberately opaque (a wrong
    /// secret and a tampered wrap are indistinguishable).
    fn verify_secret(&self, secret: &[u8]) -> bool {
        if unwrap_vek_with_secret(
            secret,
            &self.file.master_salt,
            self.file.kdf_params,
            &self.file.pw_wrap,
        )
        .is_ok()
        {
            return true;
        }
        if let Some((rec_salt, rec_wrap)) = &self.file.recovery {
            if unwrap_vek_with_secret(secret, rec_salt, self.file.kdf_params, rec_wrap).is_ok() {
                return true;
            }
        }
        false
    }

    /// Encrypt the current model under the VEK with a fresh payload nonce, rebuild
    /// the vault file preserving all header fields, and write it atomically to
    /// `path` (Req 1.5, 1.7; design "Save (any write)"). A `.bak` of the prior
    /// on-disk version is retained (Property 8).
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), VaultRepoError> {
        // Re-encrypt the payload with a fresh random nonce (never reuse a nonce
        // for a new plaintext under the same key).
        let serialized = serialize_vault(&self.model)?;
        let payload = aead_encrypt(&self.vek, &serialized, &[])?;

        // Preserve every header field; only the payload and its nonce change.
        self.file.payload_nonce = payload.nonce;
        self.file.payload = payload.ciphertext;

        atomic_write(path.as_ref(), &self.file.encode())
    }
}

/// Create a brand-new vault at `path`, protected by `master_password` and
/// (optionally) a freshly generated recovery key.
///
/// Implements the design's create flow (Req 1.5–1.8, 2.x): generate a random VEK
/// and `MASTER_SALT`, wrap the VEK under the master password, optionally wrap it
/// under a generated recovery key, encrypt the compressed empty model, and write
/// the file atomically.
///
/// Returns the [`OpenVault`] (the vault opens in an unlocked state — Req 1.7) and
/// `Some(recovery_key)` **only when** `generate_recovery` is `true`. The recovery
/// key is exposed here exactly once (Req 2.2); the caller (task 5/UI) is
/// responsible for the one-time display and is the only place it is ever shown.
pub fn create_vault(
    path: impl AsRef<Path>,
    master_password: &[u8],
    generate_recovery: bool,
    params: KdfParams,
) -> Result<(OpenVault, Option<String>), VaultRepoError> {
    let vek = random_vek();
    let master_salt = random_salt();

    // Wrap the VEK under the master-password-derived key (PW_WRAP).
    let pw_wrap = wrap_vek_with_secret(master_password, &master_salt, params, &vek)?;

    // Optionally generate a recovery key and a second, independent wrap.
    let (recovery, recovery_key) = if generate_recovery {
        let key = generate_recovery_key();
        let rec_salt = random_salt();
        let rec_wrap = wrap_vek_with_secret(key.as_bytes(), &rec_salt, params, &vek)?;
        (Some((rec_salt, rec_wrap)), Some(key))
    } else {
        (None, None)
    };

    // Encrypt the compressed, empty model under the VEK.
    let model = VaultModel::new();
    let serialized = serialize_vault(&model)?;
    let payload = aead_encrypt(&vek, &serialized, &[])?;

    let file = VaultFile::new(
        params,
        master_salt,
        pw_wrap,
        recovery,
        payload.nonce,
        payload.ciphertext,
    );

    atomic_write(path.as_ref(), &file.encode())?;

    Ok((
        OpenVault {
            vek,
            file,
            model,
            path: path.as_ref().to_path_buf(),
        },
        recovery_key,
    ))
}

/// Open and unlock a vault at `path` using the master password.
///
/// Decodes the file, unwraps the VEK from `PW_WRAP`, then decrypts and
/// deserializes the payload. An incorrect password fails with
/// [`VaultRepoError::WrongCredentials`] (Req 3.3); a payload that fails
/// authentication or parsing fails with [`VaultRepoError::Corrupted`] (Req 3.6).
pub fn unlock_with_password(
    path: impl AsRef<Path>,
    master_password: &[u8],
) -> Result<OpenVault, VaultRepoError> {
    let file = read_vault_file(path.as_ref())?;
    let vek = unwrap_vek_with_secret(
        master_password,
        &file.master_salt,
        file.kdf_params,
        &file.pw_wrap,
    )?;
    finish_unlock(path.as_ref().to_path_buf(), file, vek)
}

/// Open and unlock a vault at `path` using its recovery key.
///
/// Behaves like [`unlock_with_password`] but derives from the recovery section
/// (`REC_SALT` / `REC_WRAP`). Fails with [`VaultRepoError::NoRecoverySection`] if
/// the vault was created without a recovery key, and with
/// [`VaultRepoError::WrongCredentials`] for an incorrect recovery key (Req 2.7,
/// 2.8). The caller then prompts for a new master password (task 5.2).
pub fn unlock_with_recovery_key(
    path: impl AsRef<Path>,
    recovery_key: &[u8],
) -> Result<OpenVault, VaultRepoError> {
    let file = read_vault_file(path.as_ref())?;
    let (rec_salt, rec_wrap) = match &file.recovery {
        Some(sections) => sections.clone(),
        None => return Err(VaultRepoError::NoRecoverySection),
    };
    let vek = unwrap_vek_with_secret(recovery_key, &rec_salt, file.kdf_params, &rec_wrap)?;
    finish_unlock(path.as_ref().to_path_buf(), file, vek)
}

/// Decrypt and deserialize the payload with an already-recovered VEK, building
/// the [`OpenVault`]. A payload auth-tag failure or parse failure is reported as
/// [`VaultRepoError::Corrupted`] (the VEK was correct, so this is not a
/// credentials problem).
fn finish_unlock(
    path: PathBuf,
    file: VaultFile,
    vek: Zeroizing<[u8; VEK_LEN]>,
) -> Result<OpenVault, VaultRepoError> {
    let payload = AeadCiphertext {
        nonce: file.payload_nonce,
        ciphertext: file.payload.clone(),
    };
    let serialized =
        aead_decrypt(&vek, &payload, &[]).map_err(|_| VaultRepoError::Corrupted)?;
    let model = deserialize_vault(&serialized).map_err(|_| VaultRepoError::Corrupted)?;
    Ok(OpenVault {
        vek,
        file,
        model,
        path,
    })
}

/// Read and decode a vault file from disk into a [`VaultFile`].
fn read_vault_file(path: &Path) -> Result<VaultFile, VaultRepoError> {
    let bytes = fs::read(path)?;
    Ok(decode(&bytes)?)
}

/// Generate a human-friendly recovery key: 160 bits of CSPRNG entropy rendered
/// as Crockford base32 and grouped into dash-separated blocks of four (e.g.
/// `AB12-CD34-...`). Grouping/case are cosmetic; the exact returned string is
/// what the user re-enters to unlock via the recovery path.
fn generate_recovery_key() -> String {
    let mut raw = Zeroizing::new([0u8; RECOVERY_KEY_BYTES]);
    fill_random(raw.as_mut());
    let encoded = base32_encode(raw.as_ref());
    group_with_dashes(&encoded, 4)
}

/// Encode bytes as Crockford base32 (no padding). 5 bits per output symbol.
fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1F) as usize;
            out.push(CROCKFORD_BASE32[idx] as char);
        }
    }
    if bits > 0 {
        // Pad the final partial group on the right with zero bits.
        let idx = ((buffer << (5 - bits)) & 0x1F) as usize;
        out.push(CROCKFORD_BASE32[idx] as char);
    }
    out
}

/// Insert a `-` every `size` characters to make a long token easier to read and
/// transcribe.
fn group_with_dashes(s: &str, size: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Atomically and durably write `bytes` to `path`, retaining a `.bak` of any
/// prior version (Req 1.5, 1.7; Property 8).
///
/// Sequence — ordered so a crash at any point is recoverable:
/// 1. Write a uniquely-named temp file in the same directory and `sync_all`
///    (fsync) it. The primary file is untouched and still valid here.
/// 2. If a primary file already exists, **copy** it to its `.bak` sibling. Copy
///    (not move) keeps the primary intact throughout this step.
/// 3. `rename` the temp file over the primary. `std::fs::rename` replaces the
///    destination atomically on both Unix and Windows.
/// 4. Best-effort fsync of the containing directory (Unix only; skipped on
///    Windows where it is not reliably available).
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaultRepoError> {
    // 0. Ensure the containing directory exists. On a first run the per-user
    //    app-data directory may not have been created yet, so creating the temp
    //    file below would otherwise fail with "path not found" (os error 3).
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)?;
        }
    }

    let tmp = temp_path(path);

    // 1. Write + fsync the temp file, then close it.
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    // 2. Preserve the prior version as `.bak` (only if a primary already exists).
    if path.exists() {
        let bak = bak_path(path);
        if let Err(e) = fs::copy(path, &bak) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
    }

    // 3. Atomically replace the primary with the temp file.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }

    // 4. Best-effort directory fsync (Unix only).
    best_effort_sync_dir(path);

    Ok(())
}

/// The `.bak` sibling path for `path` (appends `.bak`, preserving the full
/// filename including its `.khv` extension).
fn bak_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// A unique temp-file path in the same directory as `path`, so the final
/// [`fs::rename`] stays within one filesystem (a precondition for atomic rename).
fn temp_path(path: &Path) -> PathBuf {
    let mut rnd = [0u8; 8];
    fill_random(&mut rnd);
    let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();

    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{suffix}.tmp"));
    path.with_file_name(name)
}

/// Best-effort fsync of the directory containing `path`, so the rename itself is
/// durable. Only attempted on Unix; on Windows directory handles cannot be
/// fsynced via the standard library, and the rename is atomic regardless.
#[cfg(unix)]
fn best_effort_sync_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(handle) = fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
}

/// No-op on non-Unix platforms (see the Unix implementation's docs).
#[cfg(not(unix))]
fn best_effort_sync_dir(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entry, Field, FieldType, ServiceRef};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Fast Argon2id parameters so the round-trip tests stay quick. Production
    /// uses [`KdfParams::recommended`]; tests must never call it (it is slow).
    fn fast_params() -> KdfParams {
        KdfParams {
            m_cost: 512,
            t_cost: 1,
            p_cost: 1,
        }
    }

    /// A unique temporary directory for a single test, removed on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let mut rnd = [0u8; 8];
            fill_random(&mut rnd);
            let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("keyhaven-test-{pid}-{n}-{suffix}"));
            fs::create_dir_all(&dir).expect("create temp dir");
            TempDir { path: dir }
        }

        fn vault_path(&self) -> PathBuf {
            self.path.join("test.khv")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn sample_entry() -> Entry {
        Entry {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            service_ref: ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            title: Some("Personal".to_string()),
            fields: vec![Field {
                id: "f1".to_string(),
                label: "Password".to_string(),
                field_type: FieldType::Password,
                value: "hunter2".to_string(),
                secret: true,
            }],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    // ---- Round-trip: create -> write -> read -> unlock (password) ----

    #[test]
    fn password_round_trip_create_unlock_persist_reopen() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let password = b"correct horse battery staple";

        // Create opens unlocked with an empty model (Req 1.7).
        let (mut vault, recovery) =
            create_vault(&path, password, false, fast_params()).expect("create");
        assert!(recovery.is_none(), "no recovery key when not requested");
        assert!(vault.model().entries.is_empty());
        assert!(path.exists(), "vault file written to disk");

        // Add an entry and persist it.
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save");

        // Reopen from disk with the password; persisted entry must be present.
        let reopened = unlock_with_password(&path, password).expect("unlock");
        assert_eq!(reopened.model().entries.len(), 1);
        assert_eq!(reopened.model().entries[0], sample_entry());
        assert!(!reopened.has_recovery());
    }

    #[test]
    fn freshly_created_vault_decrypts_to_empty_model() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let (created, _) = create_vault(&path, b"pw", false, fast_params()).expect("create");
        let opened = unlock_with_password(&path, b"pw").expect("unlock");
        assert_eq!(*created.model(), *opened.model());
        assert_eq!(*opened.model(), VaultModel::new());
    }

    // ---- Recovery path + either-key independence (Property 5, file level) ----

    #[test]
    fn recovery_round_trip_and_either_key_unlocks() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let password = b"master-password";

        let (mut vault, recovery) =
            create_vault(&path, password, true, fast_params()).expect("create");
        let recovery_key = recovery.expect("recovery key returned exactly once");
        assert!(vault.has_recovery());

        // Persist an entry so we can confirm both paths see the same data.
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save");

        // Recovery key unlocks the vault independently.
        let via_recovery =
            unlock_with_recovery_key(&path, recovery_key.as_bytes()).expect("recovery unlock");
        assert_eq!(via_recovery.model().entries, vec![sample_entry()]);

        // The master password still works after recovery exists (independence).
        let via_password = unlock_with_password(&path, password).expect("password unlock");
        assert_eq!(via_password.model().entries, vec![sample_entry()]);

        // Both paths recover the identical VEK.
        assert_eq!(via_recovery.vek(), via_password.vek());
    }

    #[test]
    fn recovery_key_is_grouped_base32() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let (_v, recovery) = create_vault(&path, b"pw", true, fast_params()).expect("create");
        let key = recovery.expect("recovery key");

        // 20 bytes -> 32 base32 symbols -> 8 groups of 4 joined by dashes.
        let groups: Vec<&str> = key.split('-').collect();
        assert_eq!(groups.len(), 8, "expected 8 dash-separated groups: {key}");
        for g in &groups {
            assert_eq!(g.len(), 4, "each group is four characters: {key}");
            assert!(
                g.bytes()
                    .all(|b| CROCKFORD_BASE32.contains(&b)),
                "only Crockford base32 symbols: {key}"
            );
        }
    }

    // ---- Wrong-secret rejection (Property 4) ----

    #[test]
    fn wrong_password_is_wrong_credentials() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        create_vault(&path, b"the-real-password", false, fast_params()).expect("create");

        let err = unlock_with_password(&path, b"not-the-password").unwrap_err();
        assert!(matches!(err, VaultRepoError::WrongCredentials), "got {err:?}");
    }

    #[test]
    fn wrong_recovery_key_is_wrong_credentials() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        create_vault(&path, b"pw", true, fast_params()).expect("create");

        let err = unlock_with_recovery_key(&path, b"WRONG-KEY-0000-0000").unwrap_err();
        assert!(matches!(err, VaultRepoError::WrongCredentials), "got {err:?}");
    }

    #[test]
    fn recovery_unlock_without_recovery_section_errors() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        // Created WITHOUT a recovery key.
        create_vault(&path, b"pw", false, fast_params()).expect("create");

        let err = unlock_with_recovery_key(&path, b"anything").unwrap_err();
        assert!(matches!(err, VaultRepoError::NoRecoverySection), "got {err:?}");
    }

    // ---- Tamper detection (Property 3) ----

    #[test]
    fn tampered_payload_byte_fails_unlock() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        create_vault(&path, b"pw", false, fast_params()).expect("create");

        // Flip the last byte (inside the payload's auth tag) and rewrite.
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&path, &bytes).unwrap();

        // VEK unwraps fine, but the payload fails authentication -> Corrupted.
        let err = unlock_with_password(&path, b"pw").unwrap_err();
        assert!(matches!(err, VaultRepoError::Corrupted), "got {err:?}");
    }

    #[test]
    fn tampered_pw_wrap_fails_unlock() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        create_vault(&path, b"pw", false, fast_params()).expect("create");

        // Corrupt a byte inside the password wrap's ciphertext. It lives right
        // after MAGIC(8)+ver(2)+cipher(1)+kdf(1)+params(12)+master_salt(16)+
        // pw_nonce(24)+pw_ct_len(4); flip the first ciphertext byte.
        let pw_ct_start = 8 + 2 + 1 + 1 + 12 + 16 + 24 + 4;
        let mut bytes = fs::read(&path).unwrap();
        bytes[pw_ct_start] ^= 0x01;
        fs::write(&path, &bytes).unwrap();

        let err = unlock_with_password(&path, b"pw").unwrap_err();
        // A tampered VEK wrap is indistinguishable from a wrong password.
        assert!(matches!(err, VaultRepoError::WrongCredentials), "got {err:?}");
    }

    // ---- Atomic write / crash safety (Property 8) ----

    #[test]
    fn save_retains_prior_version_as_bak() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let password = b"pw";

        // v1: empty vault (create writes the primary; no .bak yet).
        let (mut vault, _) = create_vault(&path, password, false, fast_params()).expect("create");
        let bak = bak_path(&path);
        assert!(!bak.exists(), "no .bak before the first overwrite");

        // v2: add an entry and save -> prior version copied to .bak.
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save v2");
        assert!(bak.exists(), ".bak retained after overwrite");

        // Primary holds v2 (one entry); .bak holds v1 (empty).
        let primary = unlock_with_password(&path, password).expect("unlock primary");
        assert_eq!(primary.model().entries.len(), 1);

        let backup = unlock_with_password(&bak, password).expect("unlock .bak");
        assert_eq!(backup.model().entries.len(), 0, ".bak holds the prior version");
    }

    #[test]
    fn corrupt_primary_recoverable_from_bak() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let password = b"pw";

        // Three states so the .bak holds a meaningful prior good version:
        // create (empty) -> save A (1 entry) -> save B (2 entries).
        // After save B: primary = 2 entries, .bak = the 1-entry version.
        let (mut vault, _) = create_vault(&path, password, false, fast_params()).expect("create");
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save A");
        let mut second = sample_entry();
        second.id = "22222222-2222-4222-8222-222222222222".to_string();
        vault.model_mut().entries.push(second);
        vault.save(&path).expect("save B");
        let bak = bak_path(&path);

        // Simulate a corrupted primary (as if a crash left it unreadable).
        fs::write(&path, b"garbage that is not a vault").unwrap();
        assert!(
            unlock_with_password(&path, password).is_err(),
            "corrupt primary must not unlock"
        );

        // The .bak still opens to the previous good version (the 1-entry state).
        let recovered = unlock_with_password(&bak, password).expect("recover from .bak");
        assert_eq!(recovered.model().entries.len(), 1);
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();

        let (mut vault, _) = create_vault(&path, b"pw", false, fast_params()).expect("create");
        vault.save(&path).expect("save");

        // After a successful create+save, the directory should contain only the
        // primary and its .bak — no leftover .tmp files.
        let leftovers: Vec<_> = fs::read_dir(&tmp.path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "stray temp files: {leftovers:?}");
    }

    #[test]
    fn create_vault_makes_missing_parent_directories() {
        // On a first run the per-user app-data directory may not exist yet.
        // create_vault must create the full parent path rather than failing
        // with "path not found" (os error 3).
        let tmp = TempDir::new();
        let path = tmp.path.join("nested").join("deeper").join("test.khv");
        assert!(!path.parent().unwrap().exists(), "precondition: dir absent");

        create_vault(&path, b"pw", false, fast_params())
            .expect("create succeeds even when parent dirs are missing");
        assert!(path.exists(), "vault file written into freshly created dirs");
    }

    #[test]
    fn save_uses_fresh_payload_nonce_each_time() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();

        let (mut vault, _) = create_vault(&path, b"pw", false, fast_params()).expect("create");
        let first_nonce = vault.file.payload_nonce;
        vault.save(&path).expect("save");
        let second_nonce = vault.file.payload_nonce;
        assert_ne!(
            first_nonce, second_nonce,
            "each save must use a fresh payload nonce"
        );
    }

    // ---- Recovery-key encoder unit tests ----

    #[test]
    fn base32_encode_is_deterministic_and_in_alphabet() {
        let data = [0u8, 1, 2, 3, 4, 250, 251, 252, 253, 254];
        let a = base32_encode(&data);
        let b = base32_encode(&data);
        assert_eq!(a, b);
        assert!(a.bytes().all(|c| CROCKFORD_BASE32.contains(&c)));
    }

    #[test]
    fn generated_recovery_keys_are_unique() {
        // Extremely unlikely to collide for 160-bit CSPRNG keys.
        let k1 = generate_recovery_key();
        let k2 = generate_recovery_key();
        assert_ne!(k1, k2);
    }

    // ---- Change master password (Property 6: change-password isolation) ----

    /// Property 6 (Change-password isolation): changing the master password
    /// rewrites only `MASTER_SALT` + `PW_WRAP`; the payload ciphertext and the
    /// recovery wrap are untouched, the old password is rejected, the new
    /// password is accepted, and the recovery key still unlocks.
    ///
    /// **Validates: Requirements 2.7, 3.2**
    #[test]
    fn change_master_password_rewrites_only_pw_wrap_and_salt() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();

        // Create with a recovery key and persist a real entry so the payload is
        // non-trivial.
        let (mut vault, recovery) =
            create_vault(&path, b"old-password", true, fast_params()).expect("create");
        let recovery_key = recovery.expect("recovery key returned once");
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save");

        // Snapshot the on-disk file immediately before the change.
        let before = decode(&fs::read(&path).unwrap()).expect("decode before");

        vault
            .change_master_password(b"old-password", b"new-password")
            .expect("change must succeed with the correct current password");

        let after = decode(&fs::read(&path).unwrap()).expect("decode after");

        // Isolation: payload (nonce + ciphertext) and recovery wrap are identical.
        assert_eq!(
            after.payload, before.payload,
            "payload ciphertext must be unchanged by a password change"
        );
        assert_eq!(
            after.payload_nonce, before.payload_nonce,
            "payload nonce must be unchanged by a password change"
        );
        assert_eq!(
            after.recovery, before.recovery,
            "recovery wrap must be unchanged by a password change"
        );
        // Only the master salt and password wrap are rewritten.
        assert_ne!(after.master_salt, before.master_salt, "master salt rotates");
        assert_ne!(after.pw_wrap, before.pw_wrap, "password wrap is rewritten");

        // Old password rejected; new password accepted; recovery still works.
        let old_err = unlock_with_password(&path, b"old-password").unwrap_err();
        assert!(matches!(old_err, VaultRepoError::WrongCredentials), "got {old_err:?}");

        let via_new = unlock_with_password(&path, b"new-password").expect("new password unlocks");
        assert_eq!(via_new.model().entries, vec![sample_entry()]);

        let via_recovery =
            unlock_with_recovery_key(&path, recovery_key.as_bytes()).expect("recovery still works");
        assert_eq!(via_recovery.model().entries, vec![sample_entry()]);
    }

    #[test]
    fn change_master_password_rejects_wrong_current_and_writes_nothing() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();

        let (mut vault, _) =
            create_vault(&path, b"old-password", true, fast_params()).expect("create");
        let before = fs::read(&path).unwrap();

        let err = vault
            .change_master_password(b"not-the-password", b"new-password")
            .unwrap_err();
        assert!(matches!(err, VaultRepoError::WrongCredentials), "got {err:?}");

        // A rejected change must not have rewritten the file at all.
        assert_eq!(fs::read(&path).unwrap(), before, "file untouched on rejection");
        // The original password still opens the vault.
        unlock_with_password(&path, b"old-password").expect("old password still valid");
    }

    /// Recovery flow (Req 2.7): after unlocking with the recovery key, the user
    /// can set a brand-new master password (supplying the recovery key as the
    /// current credential), and the recovery key still works afterward.
    #[test]
    fn recovery_unlock_then_set_new_master_password_keeps_recovery() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();

        let (mut vault, recovery) =
            create_vault(&path, b"forgotten-password", true, fast_params()).expect("create");
        let recovery_key = recovery.expect("recovery key");
        vault.model_mut().entries.push(sample_entry());
        vault.save(&path).expect("save");
        drop(vault);

        // The user forgot the master password and unlocks via the recovery key.
        let mut via_recovery =
            unlock_with_recovery_key(&path, recovery_key.as_bytes()).expect("recovery unlock");

        // They set a new master password, proving authorization with the recovery
        // key they just used.
        via_recovery
            .change_master_password(recovery_key.as_bytes(), b"brand-new-password")
            .expect("recovery flow permits setting a new master password");

        // The new password works, and the recovery key still works afterward.
        let via_new =
            unlock_with_password(&path, b"brand-new-password").expect("new password unlocks");
        assert_eq!(via_new.model().entries, vec![sample_entry()]);

        let via_recovery_again =
            unlock_with_recovery_key(&path, recovery_key.as_bytes()).expect("recovery still works");
        assert_eq!(via_recovery_again.model().entries, vec![sample_entry()]);

        // The forgotten password remains invalid.
        assert!(matches!(
            unlock_with_password(&path, b"forgotten-password").unwrap_err(),
            VaultRepoError::WrongCredentials
        ));
    }

    #[test]
    fn change_master_password_open_vault_path_tracks_source() {
        let tmp = TempDir::new();
        let path = tmp.vault_path();
        let (vault, _) = create_vault(&path, b"pw", false, fast_params()).expect("create");
        assert_eq!(vault.path(), path.as_path(), "OpenVault remembers its source path");
    }
}
