//! Vault file format and repository (skeleton).
//!
//! Implemented in task 3:
//! - Versioned binary header (MAGIC, FORMAT_VER, CIPHER_ID, KDF_ID, params,
//!   salts, wraps, payload nonce) read/write — see [`header`].
//! - Payload (de)serialization + compression of the decrypted vault model
//!   (see [`crate::model`]).
//! - Create/open flows and atomic, crash-safe writes (temp + fsync + rename,
//!   retaining a `.bak`).
//!
//! The vault is a single portable file with the `.khv` extension.

// The format/repository API is built bottom-up and is not yet referenced from
// the binary, consistent with the other core modules.
#![allow(dead_code)]

pub mod header;
pub mod repository;

// Re-exported for the create/open flows (task 3.3) and command layer (task 5).
#[allow(unused_imports)]
pub use header::{decode, VaultFile, VaultFormatError, FORMAT_VERSION, MAGIC};

// The repository surface consumed by the session manager (task 4) and the
// command layer (task 5): create/open flows, the unlocked handle, and the
// composed error type.
#[allow(unused_imports)]
pub use repository::{
    create_vault, unlock_with_password, unlock_with_recovery_key, OpenVault, VaultRepoError,
};
