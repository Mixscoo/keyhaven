//! The shared `KeyhavenError` type — the single error surface returned across
//! the Tauri IPC boundary.
//!
//! The design specifies one error enum mapped to user-friendly messages in the
//! frontend. Lower layers (crypto, vault, model) keep their own focused error
//! types; the command layer (tasks 5+) maps those onto these variants. This
//! type is introduced here because the session manager (task 4.1) needs the
//! [`KeyhavenError::Locked`] variant to gate entry commands when the vault is
//! locked.
//!
//! The enum derives [`serde::Serialize`] so command handlers can return
//! `Result<T, KeyhavenError>` directly: Tauri serializes the error to the
//! frontend, which switches on the stable `code` discriminant.

// Some variants are produced by later tasks (the command-layer mappings in
// tasks 5+); only `Locked` is exercised by task 4.1.
#![allow(dead_code)]

use serde::Serialize;

/// All error conditions surfaced to the frontend over IPC.
///
/// Serializes as an internally-tagged object keyed by `code` (camelCase), e.g.
/// `{ "code": "locked" }` or `{ "code": "io", "message": "..." }`. The codes are
/// part of the IPC contract; the frontend maps them to user-facing copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum KeyhavenError {
    /// The supplied master password or recovery key could not unwrap the VEK
    /// (AEAD auth-tag failure). Deliberately opaque about which secret/why.
    WrongCredentials,
    /// The vault file is damaged, truncated, or not a Keyhaven vault (bad magic
    /// or a payload that fails authentication).
    VaultCorrupted,
    /// The vault's file-format version is newer than this build supports.
    IncompatibleVersion,
    /// An operation requiring an unlocked vault was attempted while locked.
    /// The frontend routes back to the Unlock screen.
    Locked,
    /// A filesystem error (read/write/permission). Carries a human-readable
    /// message including context such as the path.
    Io {
        /// Human-readable description of the I/O failure.
        message: String,
    },
    /// A command received invalid arguments. Carries an inline validation
    /// message for the frontend to display.
    InvalidInput {
        /// Human-readable description of what was invalid.
        message: String,
    },
}

impl std::fmt::Display for KeyhavenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyhavenError::WrongCredentials => {
                write!(f, "incorrect master password or recovery key")
            }
            KeyhavenError::VaultCorrupted => {
                write!(f, "this vault file is damaged or not a Keyhaven vault")
            }
            KeyhavenError::IncompatibleVersion => {
                write!(f, "this vault was created by a newer version of Keyhaven")
            }
            KeyhavenError::Locked => write!(f, "the vault is locked"),
            KeyhavenError::Io { message } => write!(f, "file error: {message}"),
            KeyhavenError::InvalidInput { message } => write!(f, "invalid input: {message}"),
        }
    }
}

impl std::error::Error for KeyhavenError {}

/// Map an entry-repository [`EntryError`](crate::entries::EntryError) onto the
/// IPC-facing [`KeyhavenError`]. A missing entry id is a client/validation
/// problem (the entry was already deleted, or the id is bogus), so it surfaces
/// as [`KeyhavenError::InvalidInput`] with an inline message.
impl From<crate::entries::EntryError> for KeyhavenError {
    fn from(e: crate::entries::EntryError) -> Self {
        match e {
            crate::entries::EntryError::NotFound => KeyhavenError::InvalidInput {
                message: "no entry with that id exists".to_string(),
            },
        }
    }
}

/// Map the vault repository's composed error onto the single IPC-facing
/// [`KeyhavenError`]. This is the command-layer mapping the design calls for:
/// lower layers keep focused error types and the commands (tasks 5+) translate
/// them here so the frontend only ever sees the stable `code` discriminants.
///
/// - A wrong master password / recovery key (and the indistinguishable
///   tampered-wrap case) becomes [`KeyhavenError::WrongCredentials`].
/// - A recovery-unlock attempt against a vault with no recovery section also
///   surfaces as `WrongCredentials`: from the user's perspective the supplied
///   recovery key cannot open this vault.
/// - A damaged payload or a structurally invalid file becomes
///   [`KeyhavenError::VaultCorrupted`]; a too-new file becomes
///   [`KeyhavenError::IncompatibleVersion`].
/// - Filesystem failures become [`KeyhavenError::Io`].
/// - The remaining internal crypto/model/envelope failures are
///   "should-not-happen" runtime errors; they are surfaced as
///   [`KeyhavenError::Io`] carrying their descriptive message rather than being
///   silently swallowed.
impl From<crate::vault::VaultRepoError> for KeyhavenError {
    fn from(e: crate::vault::VaultRepoError) -> Self {
        use crate::vault::{VaultFormatError, VaultRepoError};
        match e {
            VaultRepoError::WrongCredentials | VaultRepoError::NoRecoverySection => {
                KeyhavenError::WrongCredentials
            }
            VaultRepoError::Corrupted
            | VaultRepoError::Format(VaultFormatError::VaultCorrupted) => {
                KeyhavenError::VaultCorrupted
            }
            VaultRepoError::Format(VaultFormatError::IncompatibleVersion { .. }) => {
                KeyhavenError::IncompatibleVersion
            }
            VaultRepoError::Format(VaultFormatError::Io(err)) => KeyhavenError::Io {
                message: err.to_string(),
            },
            VaultRepoError::Io(err) => KeyhavenError::Io {
                message: err.to_string(),
            },
            VaultRepoError::Envelope(err) => KeyhavenError::Io {
                message: err.to_string(),
            },
            VaultRepoError::Model(err) => KeyhavenError::Io {
                message: err.to_string(),
            },
            VaultRepoError::Crypto(err) => KeyhavenError::Io {
                message: err.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_serializes_to_stable_code() {
        let json = serde_json::to_value(KeyhavenError::Locked).unwrap();
        assert_eq!(json["code"], "locked");
    }

    #[test]
    fn io_variant_carries_message() {
        let json = serde_json::to_value(KeyhavenError::Io {
            message: "permission denied".to_string(),
        })
        .unwrap();
        assert_eq!(json["code"], "io");
        assert_eq!(json["message"], "permission denied");
    }

    #[test]
    fn wrong_credentials_code_is_camel_case() {
        let json = serde_json::to_value(KeyhavenError::WrongCredentials).unwrap();
        assert_eq!(json["code"], "wrongCredentials");
    }

    #[test]
    fn display_messages_are_user_friendly() {
        assert_eq!(KeyhavenError::Locked.to_string(), "the vault is locked");
        assert!(KeyhavenError::WrongCredentials
            .to_string()
            .contains("incorrect"));
    }

    #[test]
    fn repo_error_maps_to_keyhaven_error() {
        use crate::vault::{VaultFormatError, VaultRepoError};

        assert_eq!(
            KeyhavenError::from(VaultRepoError::WrongCredentials),
            KeyhavenError::WrongCredentials
        );
        // A recovery key on a vault with no recovery section is, to the user, a
        // credentials failure.
        assert_eq!(
            KeyhavenError::from(VaultRepoError::NoRecoverySection),
            KeyhavenError::WrongCredentials
        );
        assert_eq!(
            KeyhavenError::from(VaultRepoError::Corrupted),
            KeyhavenError::VaultCorrupted
        );
        assert_eq!(
            KeyhavenError::from(VaultRepoError::Format(VaultFormatError::VaultCorrupted)),
            KeyhavenError::VaultCorrupted
        );
        assert_eq!(
            KeyhavenError::from(VaultRepoError::Format(
                VaultFormatError::IncompatibleVersion {
                    found: 2,
                    supported: 1,
                }
            )),
            KeyhavenError::IncompatibleVersion
        );
        let io = KeyhavenError::from(VaultRepoError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        )));
        assert!(matches!(io, KeyhavenError::Io { .. }));
    }
}
