//! Envelope encryption: wrapping and unwrapping the Vault Encryption Key (VEK).
//!
//! Keyhaven uses **envelope encryption** (Req 2.3): a single random VEK
//! encrypts the vault payload, and the VEK itself is encrypted ("wrapped")
//! separately under one or more keys derived from user secrets — the master
//! password (Argon2id over `MASTER_SALT`) and, optionally, a recovery key
//! (Argon2id over `REC_SALT`). Either wrap can independently recover the same
//! VEK, which is what lets either secret unlock the vault and lets the master
//! password change without re-encrypting the payload.
//!
//! A "wrap" is simply an [`AeadCiphertext`] produced by AEAD-encrypting the
//! 32-byte VEK under a derived key (built on [`crate::crypto::aead_encrypt`]).
//! Unwrapping AEAD-decrypts it; because XChaCha20-Poly1305 is authenticated, a
//! wrong derived key (wrong password/recovery key) or a tampered wrap fails the
//! authentication tag and never yields a usable VEK (Req 2.8).
//!
//! ## Error handling decision
//!
//! This layer exposes [`EnvelopeError`] rather than reusing
//! [`CryptoError`](crate::crypto::CryptoError) directly. The key distinction is
//! [`EnvelopeError::WrongCredentials`]: an AEAD authentication failure during
//! unwrapping is mapped to `WrongCredentials` so the command layer can surface
//! the design's `KeyhavenError::WrongCredentials` ("Incorrect master password /
//! recovery key.") without inspecting opaque crypto errors.
//!
//! `WrongCredentials` is intentionally **opaque**: a wrong key and a tampered
//! wrap both surface the same variant, so it cannot be used as an oracle to
//! distinguish "wrong key" from "tampered data" (mirroring the deliberate
//! opacity of [`CryptoError::Decryption`](crate::crypto::CryptoError::Decryption)).
//! Other failures (key derivation, encryption, an unexpected unwrapped length)
//! are carried distinctly so genuine misconfiguration is not masked as a
//! credentials error.
//!
//! All keys and the VEK are handled inside [`Zeroizing`] so they are cleared
//! from memory when dropped (Req 15.5). The vault file never stores the VEK,
//! derived keys, or secrets in plaintext — only the wrapped VEK is persisted
//! (Req 15.3).

use zeroize::Zeroizing;

use super::{aead_decrypt, aead_encrypt, derive_key, AeadCiphertext, CryptoError, KdfParams, VEK_LEN};

/// Errors that can occur while wrapping or unwrapping the VEK.
///
/// See the module docs for the rationale behind [`EnvelopeError::WrongCredentials`]
/// and its deliberate opacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The supplied secret/derived key could not unwrap the VEK: the AEAD
    /// authentication tag failed. This is the expected outcome for an incorrect
    /// master password or recovery key, and also for a tampered wrap — the two
    /// are intentionally indistinguishable. The command layer maps this to
    /// `KeyhavenError::WrongCredentials`.
    WrongCredentials,
    /// An underlying cryptographic operation failed for a reason unrelated to
    /// credentials (e.g. key derivation with invalid parameters, or AEAD
    /// encryption while wrapping).
    Crypto(CryptoError),
    /// Unwrapping succeeded cryptographically but did not yield exactly
    /// [`VEK_LEN`] bytes. This indicates a malformed wrap rather than a wrong
    /// key, so it is reported distinctly.
    InvalidVek,
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeError::WrongCredentials => {
                write!(f, "incorrect master password or recovery key")
            }
            EnvelopeError::Crypto(e) => write!(f, "envelope crypto error: {e}"),
            EnvelopeError::InvalidVek => write!(f, "unwrapped key has an invalid length"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl From<CryptoError> for EnvelopeError {
    fn from(e: CryptoError) -> Self {
        EnvelopeError::Crypto(e)
    }
}

/// Wrap (AEAD-encrypt) the 32-byte `vek` under an already-derived `derived_key`.
///
/// The result is an [`AeadCiphertext`] (`{ nonce, ciphertext }`) suitable for
/// storage as `PW_WRAP` or `REC_WRAP` in the vault header. Use this when the
/// caller has already derived the key (e.g. to reuse a derivation); otherwise
/// prefer [`wrap_vek_with_secret`].
pub fn wrap_vek(
    derived_key: &[u8; VEK_LEN],
    vek: &[u8; VEK_LEN],
) -> Result<AeadCiphertext, EnvelopeError> {
    Ok(aead_encrypt(derived_key, vek.as_slice(), &[])?)
}

/// Unwrap (AEAD-decrypt) a VEK wrap using an already-derived `derived_key`.
///
/// Returns the recovered VEK in [`Zeroizing`]. An authentication-tag failure
/// (wrong derived key or tampered wrap) maps to
/// [`EnvelopeError::WrongCredentials`]; no plaintext is ever returned on
/// failure. A successful decryption that is not exactly [`VEK_LEN`] bytes maps
/// to [`EnvelopeError::InvalidVek`].
pub fn unwrap_vek(
    derived_key: &[u8; VEK_LEN],
    wrap: &AeadCiphertext,
) -> Result<Zeroizing<[u8; VEK_LEN]>, EnvelopeError> {
    let plaintext = match aead_decrypt(derived_key, wrap, &[]) {
        Ok(pt) => pt,
        // Opaque: wrong key and tamper both surface as WrongCredentials.
        Err(CryptoError::Decryption) => return Err(EnvelopeError::WrongCredentials),
        Err(other) => return Err(EnvelopeError::Crypto(other)),
    };

    to_vek_array(&plaintext)
}

/// Derive a key from `secret` + `salt` (Argon2id with `params`) and wrap the
/// `vek` under it in one step.
///
/// This is the convenience entry point for the vault create flow (task 5.1):
/// it builds the password wrap (`secret = master password`, `salt = MASTER_SALT`)
/// and, when a recovery key is requested, the recovery wrap
/// (`secret = recovery key`, `salt = REC_SALT`). The derived key lives in
/// [`Zeroizing`] and is cleared when this function returns.
pub fn wrap_vek_with_secret(
    secret: &[u8],
    salt: &[u8],
    params: KdfParams,
    vek: &[u8; VEK_LEN],
) -> Result<AeadCiphertext, EnvelopeError> {
    let derived = derive_key(secret, salt, params)?;
    wrap_vek(&derived, vek)
}

/// Derive a key from `secret` + `salt` (Argon2id with `params`) and unwrap the
/// VEK from `wrap` in one step.
///
/// This is the convenience entry point for the unlock flows (task 5.2): the
/// password path passes the master password + `MASTER_SALT` + `PW_WRAP`, and the
/// recovery path passes the recovery key + `REC_SALT` + `REC_WRAP`. A wrong
/// secret fails with [`EnvelopeError::WrongCredentials`]. The derived key is
/// zeroized when this function returns; the recovered VEK is returned in
/// [`Zeroizing`].
pub fn unwrap_vek_with_secret(
    secret: &[u8],
    salt: &[u8],
    params: KdfParams,
    wrap: &AeadCiphertext,
) -> Result<Zeroizing<[u8; VEK_LEN]>, EnvelopeError> {
    let derived = derive_key(secret, salt, params)?;
    unwrap_vek(&derived, wrap)
}

/// Convert an unwrapped plaintext into a fixed-size, zeroizing VEK array,
/// validating the length. The intermediate `Zeroizing<Vec<u8>>` is cleared on
/// drop, and the copied bytes live only inside the returned `Zeroizing` array.
fn to_vek_array(plaintext: &[u8]) -> Result<Zeroizing<[u8; VEK_LEN]>, EnvelopeError> {
    if plaintext.len() != VEK_LEN {
        return Err(EnvelopeError::InvalidVek);
    }
    let mut vek = Zeroizing::new([0u8; VEK_LEN]);
    vek.copy_from_slice(plaintext);
    Ok(vek)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{random_salt, random_vek};

    /// Cheap KDF parameters so tests stay fast while still exercising Argon2id.
    fn fast_params() -> KdfParams {
        KdfParams {
            m_cost: 512, // 512 KiB
            t_cost: 1,
            p_cost: 1,
        }
    }

    #[test]
    fn wrap_then_unwrap_round_trip_recovers_exact_vek() {
        let derived = [42u8; VEK_LEN];
        let vek = random_vek();

        let wrap = wrap_vek(&derived, &vek).unwrap();
        let recovered = unwrap_vek(&derived, &wrap).unwrap();

        assert_eq!(*recovered, *vek, "unwrapped VEK must equal the original");
        // A wrap must not expose the VEK in the clear, and carries the 16-byte tag.
        assert_ne!(&wrap.ciphertext[..], vek.as_slice());
        assert_eq!(wrap.ciphertext.len(), VEK_LEN + 16);
    }

    #[test]
    fn unwrap_with_correct_key_succeeds() {
        let derived = [7u8; VEK_LEN];
        let vek = random_vek();

        let wrap = wrap_vek(&derived, &vek).unwrap();
        let recovered = unwrap_vek(&derived, &wrap).expect("correct key must unwrap");

        assert_eq!(*recovered, *vek);
    }

    #[test]
    fn unwrap_with_wrong_key_returns_wrong_credentials() {
        let derived = [1u8; VEK_LEN];
        let wrong = [2u8; VEK_LEN];
        let vek = random_vek();

        let wrap = wrap_vek(&derived, &vek).unwrap();
        let err = unwrap_vek(&wrong, &wrap).unwrap_err();

        assert_eq!(err, EnvelopeError::WrongCredentials);
    }

    #[test]
    fn tampered_wrap_fails_as_wrong_credentials() {
        let derived = [8u8; VEK_LEN];
        let vek = random_vek();

        // Tamper with the ciphertext.
        let mut wrap = wrap_vek(&derived, &vek).unwrap();
        wrap.ciphertext[0] ^= 0x01;
        assert_eq!(
            unwrap_vek(&derived, &wrap).unwrap_err(),
            EnvelopeError::WrongCredentials,
            "a tampered wrap must be rejected and indistinguishable from a wrong key"
        );

        // Tamper with the nonce.
        let mut wrap2 = wrap_vek(&derived, &vek).unwrap();
        wrap2.nonce[0] ^= 0x01;
        assert_eq!(
            unwrap_vek(&derived, &wrap2).unwrap_err(),
            EnvelopeError::WrongCredentials
        );
    }

    #[test]
    fn wrap_unwrap_with_secret_round_trip() {
        let secret = b"master password";
        let salt = random_salt();
        let params = fast_params();
        let vek = random_vek();

        let wrap = wrap_vek_with_secret(secret, &salt, params, &vek).unwrap();
        let recovered = unwrap_vek_with_secret(secret, &salt, params, &wrap).unwrap();

        assert_eq!(*recovered, *vek);
    }

    #[test]
    fn unwrap_with_secret_wrong_password_returns_wrong_credentials() {
        let salt = random_salt();
        let params = fast_params();
        let vek = random_vek();

        let wrap = wrap_vek_with_secret(b"correct password", &salt, params, &vek).unwrap();
        let err = unwrap_vek_with_secret(b"wrong password", &salt, params, &wrap).unwrap_err();

        assert_eq!(err, EnvelopeError::WrongCredentials);
    }

    /// Property 5 (Either-key independence): with both a password wrap and a
    /// recovery wrap over the same VEK, each secret independently recovers the
    /// exact same VEK, and neither requires the other.
    ///
    /// **Validates: Requirements 2.3, 2.7**
    #[test]
    fn either_key_independently_recovers_same_vek() {
        let master_password = b"the master password";
        let recovery_key = b"RECOVERY-KEY-ABCD-1234-WXYZ";
        let master_salt = random_salt();
        let rec_salt = random_salt();
        let params = fast_params();

        // One VEK, wrapped twice under two independently derived keys.
        let vek = random_vek();
        let pw_wrap = wrap_vek_with_secret(master_password, &master_salt, params, &vek).unwrap();
        let rec_wrap = wrap_vek_with_secret(recovery_key, &rec_salt, params, &vek).unwrap();

        // Password path recovers the VEK on its own.
        let from_pw = unwrap_vek_with_secret(master_password, &master_salt, params, &pw_wrap)
            .expect("master password must unwrap the password wrap");
        // Recovery path recovers the VEK on its own.
        let from_rec = unwrap_vek_with_secret(recovery_key, &rec_salt, params, &rec_wrap)
            .expect("recovery key must unwrap the recovery wrap");

        assert_eq!(*from_pw, *vek, "password wrap must recover the original VEK");
        assert_eq!(*from_rec, *vek, "recovery wrap must recover the original VEK");
        assert_eq!(*from_pw, *from_rec, "both paths must yield the identical VEK");
    }

    /// Property 4 (Wrong-secret rejection): unwrapping with an incorrect secret
    /// fails and never yields a usable VEK — confirmed here against a recovery
    /// wrap as well as the password wrap.
    ///
    /// **Validates: Requirements 2.8, 3.3**
    #[test]
    fn wrong_secret_never_unwraps_either_wrap() {
        let master_password = b"the master password";
        let recovery_key = b"RECOVERY-KEY-ABCD-1234-WXYZ";
        let master_salt = random_salt();
        let rec_salt = random_salt();
        let params = fast_params();

        let vek = random_vek();
        let pw_wrap = wrap_vek_with_secret(master_password, &master_salt, params, &vek).unwrap();
        let rec_wrap = wrap_vek_with_secret(recovery_key, &rec_salt, params, &vek).unwrap();

        // Recovery key cannot open the password wrap (different salt + secret).
        assert_eq!(
            unwrap_vek_with_secret(recovery_key, &master_salt, params, &pw_wrap).unwrap_err(),
            EnvelopeError::WrongCredentials
        );
        // Master password cannot open the recovery wrap.
        assert_eq!(
            unwrap_vek_with_secret(master_password, &rec_salt, params, &rec_wrap).unwrap_err(),
            EnvelopeError::WrongCredentials
        );
        // An entirely unrelated guess opens neither.
        assert_eq!(
            unwrap_vek_with_secret(b"guess", &master_salt, params, &pw_wrap).unwrap_err(),
            EnvelopeError::WrongCredentials
        );
        assert_eq!(
            unwrap_vek_with_secret(b"guess", &rec_salt, params, &rec_wrap).unwrap_err(),
            EnvelopeError::WrongCredentials
        );
    }

    #[test]
    fn invalid_length_plaintext_maps_to_invalid_vek() {
        // Encrypt a non-VEK-length plaintext, then unwrap: the crypto succeeds
        // but the length check must reject it as InvalidVek (not WrongCredentials).
        let derived = [3u8; VEK_LEN];
        let not_a_vek = aead_encrypt(&derived, b"too short", &[]).unwrap();

        assert_eq!(
            unwrap_vek(&derived, &not_a_vek).unwrap_err(),
            EnvelopeError::InvalidVek
        );
    }

    #[test]
    fn crypto_error_converts_into_envelope_error() {
        let e: EnvelopeError = CryptoError::Encryption.into();
        assert_eq!(e, EnvelopeError::Crypto(CryptoError::Encryption));
    }
}
