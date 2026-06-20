//! Cryptographic primitives for Keyhaven.
//!
//! This module is the cryptographic trust boundary. It exposes a small,
//! well-documented surface built **entirely on audited crates** — no custom
//! cryptography is implemented here (Req 15.6):
//!
//! - **KDF**: Argon2id key derivation ([`derive_key`]) tuned to resist
//!   brute-force attacks (Req 15.2).
//! - **AEAD**: XChaCha20-Poly1305 authenticated encryption
//!   ([`aead_encrypt`] / [`aead_decrypt`]) with a 24-byte random nonce
//!   (Req 15.1). Authentication tags make tampering detectable (Req 15.4).
//! - **CSPRNG helpers**: [`random_salt`], [`random_nonce`], [`random_vek`]
//!   backed by the OS CSPRNG via `getrandom`/`OsRng`.
//!
//! All derived keys and the Vault Encryption Key (VEK) are wrapped in
//! [`Zeroizing`] so they are cleared from memory when dropped (Req 15.5).
//!
//! Task 2.2 (envelope encryption) builds on these primitives: the VEK is
//! produced by [`random_vek`] and wrapped/unwrapped using [`aead_encrypt`] /
//! [`aead_decrypt`].

// These primitives form the public crypto API consumed by later tasks
// (envelope encryption, vault format). Some items are not yet referenced from
// the binary, which is expected while the core is built bottom-up.
#![allow(dead_code)]

pub mod envelope;

// Re-exported for the vault format and command layers (tasks 3 & 5); not yet
// referenced from the binary while the core is built bottom-up.
#[allow(unused_imports)]
pub use envelope::{
    unwrap_vek, unwrap_vek_with_secret, wrap_vek, wrap_vek_with_secret, EnvelopeError,
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand::{rngs::OsRng, RngCore};
use zeroize::Zeroizing;

// ---- Algorithm identifiers (recorded in the vault header) ----

/// Cipher identifier for XChaCha20-Poly1305 (stored as `CIPHER_ID`).
pub const CIPHER_ID_XCHACHA20POLY1305: u8 = 1;
/// KDF identifier for Argon2id (stored as `KDF_ID`).
pub const KDF_ID_ARGON2ID: u8 = 1;

// ---- Fixed sizes ----

/// Length of a symmetric key / derived key / VEK, in bytes (256-bit).
pub const KEY_LEN: usize = 32;
/// Length of a KDF salt, in bytes (128-bit).
pub const SALT_LEN: usize = 16;
/// Length of an XChaCha20-Poly1305 nonce, in bytes (192-bit).
pub const NONCE_LEN: usize = 24;
/// Length of the Vault Encryption Key, in bytes (256-bit).
pub const VEK_LEN: usize = KEY_LEN;

/// Errors that can occur in the cryptographic core.
///
/// `Decryption` is deliberately opaque: a wrong key, a tampered ciphertext,
/// and a tampered nonce all surface the same error so callers cannot use it as
/// an oracle to distinguish failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    /// Argon2id key derivation failed (e.g. invalid parameters).
    KeyDerivation(String),
    /// Provided KDF parameters were invalid.
    InvalidParams(String),
    /// AEAD encryption failed.
    Encryption,
    /// AEAD decryption/authentication failed (wrong key or tampered data).
    Decryption,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::KeyDerivation(msg) => write!(f, "key derivation failed: {msg}"),
            CryptoError::InvalidParams(msg) => write!(f, "invalid KDF parameters: {msg}"),
            CryptoError::Encryption => write!(f, "encryption failed"),
            CryptoError::Decryption => write!(f, "decryption failed (wrong key or tampered data)"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// Argon2id key-derivation parameters.
///
/// These are stored in the vault header so a vault remains openable even if the
/// application's defaults change later. Costs follow the `argon2` crate units:
/// - `m_cost`: memory size in **KiB**
/// - `t_cost`: number of iterations (time cost)
/// - `p_cost`: degree of parallelism (lanes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory size in KiB.
    pub m_cost: u32,
    /// Number of iterations.
    pub t_cost: u32,
    /// Degree of parallelism.
    pub p_cost: u32,
}

impl KdfParams {
    /// Tuned default parameters targeting roughly ~250-500ms of derivation
    /// time on typical consumer hardware: 64 MiB memory, 3 iterations, 1 lane.
    ///
    /// These exceed the OWASP Argon2id minimum (19 MiB, t=2, p=1) for a
    /// stronger brute-force resistance margin (Req 15.2).
    pub const fn recommended() -> Self {
        KdfParams {
            m_cost: 64 * 1024, // 64 MiB
            t_cost: 3,
            p_cost: 1,
        }
    }

    fn to_argon2_params(self) -> Result<Params, CryptoError> {
        Params::new(self.m_cost, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|e| CryptoError::InvalidParams(e.to_string()))
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams::recommended()
    }
}

/// An AEAD ciphertext together with the random nonce used to produce it.
///
/// This is the unit of encrypted data used throughout the vault format: each
/// wrapped key and the payload are stored as one of these `{ nonce, ciphertext }`
/// pairs. The `ciphertext` includes the Poly1305 authentication tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AeadCiphertext {
    /// The 24-byte XChaCha20-Poly1305 nonce.
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext with the appended 16-byte authentication tag.
    pub ciphertext: Vec<u8>,
}

/// Derive a 256-bit key from `secret` and `salt` using Argon2id.
///
/// Deterministic: the same `secret`, `salt`, and `params` always produce the
/// same key. The returned key is wrapped in [`Zeroizing`] so it is cleared from
/// memory when dropped (Req 15.5).
pub fn derive_key(
    secret: &[u8],
    salt: &[u8],
    params: KdfParams,
) -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.to_argon2_params()?);

    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(secret, salt, key.as_mut())
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

    Ok(key)
}

/// Encrypt `plaintext` with XChaCha20-Poly1305 under `key`, generating a fresh
/// random 24-byte nonce.
///
/// `aad` is additional authenticated data: it is not encrypted but is
/// integrity-protected (it must match on decryption). Pass `&[]` when no
/// associated data is needed. The large random nonce removes the need for nonce
/// counter management and makes accidental nonce reuse vanishingly unlikely.
pub fn aead_encrypt(
    key: &[u8; KEY_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<AeadCiphertext, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = random_nonce();

    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encryption)?;

    Ok(AeadCiphertext { nonce, ciphertext })
}

/// Decrypt an [`AeadCiphertext`] with XChaCha20-Poly1305 under `key`.
///
/// Returns the plaintext wrapped in [`Zeroizing`]. Fails with
/// [`CryptoError::Decryption`] if the key is wrong, the `aad` differs, or the
/// nonce/ciphertext/tag has been tampered with — the authentication tag is
/// verified before any plaintext is returned (Req 15.4).
pub fn aead_decrypt(
    key: &[u8; KEY_LEN],
    ct: &AeadCiphertext,
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());

    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&ct.nonce),
            Payload {
                msg: &ct.ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Decryption)?;

    Ok(Zeroizing::new(plaintext))
}

/// Fill `buf` with cryptographically secure random bytes from the OS CSPRNG.
pub fn fill_random(buf: &mut [u8]) {
    OsRng.fill_bytes(buf);
}

/// Generate a random 16-byte KDF salt.
pub fn random_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    fill_random(&mut salt);
    salt
}

/// Generate a random 24-byte XChaCha20-Poly1305 nonce.
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    fill_random(&mut nonce);
    nonce
}

/// Generate a random 256-bit Vault Encryption Key (VEK).
///
/// Returned in [`Zeroizing`] so it is cleared from memory when dropped.
pub fn random_vek() -> Zeroizing<[u8; VEK_LEN]> {
    let mut vek = Zeroizing::new([0u8; VEK_LEN]);
    fill_random(vek.as_mut());
    vek
}

/// Return a uniformly distributed random `u32` in the half-open range
/// `[0, bound)` using the OS CSPRNG (the same `getrandom`/`OsRng` source as the
/// rest of this module).
///
/// Uses **rejection sampling** to avoid the modulo bias that a naive
/// `random() % bound` would introduce: any raw draw landing in the short
/// "remainder" zone above the largest multiple of `bound` is discarded and
/// redrawn, so every output value is equally likely. This is the CSPRNG-backed
/// primitive the password generator (task 8.1) uses to pick characters without
/// skewing the distribution toward lower indices.
///
/// # Panics
/// Panics if `bound == 0` (an empty range has no valid output).
pub fn random_below(bound: u32) -> u32 {
    assert!(bound > 0, "bound must be non-zero");

    // Work in u64 to size the acceptance window without overflowing: there are
    // 2^32 possible raw draws; accept only those below the largest multiple of
    // `bound` that fits, guaranteeing a uniform mapping under the modulo.
    let bound = u64::from(bound);
    let span = 1u64 << 32;
    let limit = span - (span % bound);

    loop {
        let mut buf = [0u8; 4];
        fill_random(&mut buf);
        let v = u64::from(u32::from_le_bytes(buf));
        if v < limit {
            return (v % bound) as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap KDF parameters so tests stay fast while still exercising Argon2id.
    fn fast_params() -> KdfParams {
        KdfParams {
            m_cost: 512, // 512 KiB
            t_cost: 1,
            p_cost: 1,
        }
    }

    #[test]
    fn derive_key_is_deterministic() {
        let secret = b"correct horse battery staple";
        let salt = [7u8; SALT_LEN];
        let params = fast_params();

        let k1 = derive_key(secret, &salt, params).unwrap();
        let k2 = derive_key(secret, &salt, params).unwrap();

        assert_eq!(*k1, *k2, "same secret+salt+params must yield the same key");
        assert_eq!(k1.len(), KEY_LEN);
    }

    #[test]
    fn derive_key_differs_by_salt() {
        let secret = b"correct horse battery staple";
        let params = fast_params();

        let k1 = derive_key(secret, &[1u8; SALT_LEN], params).unwrap();
        let k2 = derive_key(secret, &[2u8; SALT_LEN], params).unwrap();

        assert_ne!(*k1, *k2, "different salts must yield different keys");
    }

    #[test]
    fn derive_key_differs_by_secret() {
        let salt = [9u8; SALT_LEN];
        let params = fast_params();

        let k1 = derive_key(b"password-one", &salt, params).unwrap();
        let k2 = derive_key(b"password-two", &salt, params).unwrap();

        assert_ne!(*k1, *k2, "different secrets must yield different keys");
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = [42u8; KEY_LEN];
        let plaintext = b"top secret vault payload";

        let ct = aead_encrypt(&key, plaintext, &[]).unwrap();
        let recovered = aead_decrypt(&key, &ct, &[]).unwrap();

        assert_eq!(&recovered[..], plaintext);
        // Ciphertext must not equal plaintext, and carries the 16-byte tag.
        assert_ne!(&ct.ciphertext[..], &plaintext[..]);
        assert_eq!(ct.ciphertext.len(), plaintext.len() + 16);
        assert_eq!(ct.nonce.len(), NONCE_LEN);
    }

    #[test]
    fn round_trip_with_aad() {
        let key = [3u8; KEY_LEN];
        let plaintext = b"payload bound to header";
        let aad = b"vault-header-bytes";

        let ct = aead_encrypt(&key, plaintext, aad).unwrap();
        let recovered = aead_decrypt(&key, &ct, aad).unwrap();
        assert_eq!(&recovered[..], plaintext);
    }

    #[test]
    fn round_trip_empty_plaintext() {
        let key = [5u8; KEY_LEN];
        let ct = aead_encrypt(&key, b"", &[]).unwrap();
        let recovered = aead_decrypt(&key, &ct, &[]).unwrap();
        assert!(recovered.is_empty());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let key = [1u8; KEY_LEN];
        let wrong_key = [2u8; KEY_LEN];
        let plaintext = b"secret";

        let ct = aead_encrypt(&key, plaintext, &[]).unwrap();
        let result = aead_decrypt(&wrong_key, &ct, &[]);

        assert_eq!(result.unwrap_err(), CryptoError::Decryption);
    }

    #[test]
    fn mismatched_aad_is_rejected() {
        let key = [1u8; KEY_LEN];
        let ct = aead_encrypt(&key, b"secret", b"aad-a").unwrap();
        let result = aead_decrypt(&key, &ct, b"aad-b");
        assert_eq!(result.unwrap_err(), CryptoError::Decryption);
    }

    #[test]
    fn tampered_ciphertext_byte_fails_auth() {
        let key = [8u8; KEY_LEN];
        let plaintext = b"integrity matters";

        let mut ct = aead_encrypt(&key, plaintext, &[]).unwrap();
        // Flip a single bit in the first ciphertext byte.
        ct.ciphertext[0] ^= 0x01;

        let result = aead_decrypt(&key, &ct, &[]);
        assert_eq!(result.unwrap_err(), CryptoError::Decryption);
    }

    #[test]
    fn tampered_tag_fails_auth() {
        let key = [8u8; KEY_LEN];
        let mut ct = aead_encrypt(&key, b"integrity matters", &[]).unwrap();
        // Flip a bit in the last byte (within the authentication tag).
        let last = ct.ciphertext.len() - 1;
        ct.ciphertext[last] ^= 0x80;

        let result = aead_decrypt(&key, &ct, &[]);
        assert_eq!(result.unwrap_err(), CryptoError::Decryption);
    }

    #[test]
    fn tampered_nonce_fails_auth() {
        let key = [8u8; KEY_LEN];
        let mut ct = aead_encrypt(&key, b"integrity matters", &[]).unwrap();
        ct.nonce[0] ^= 0x01;

        let result = aead_decrypt(&key, &ct, &[]);
        assert_eq!(result.unwrap_err(), CryptoError::Decryption);
    }

    #[test]
    fn nonces_are_random_per_encryption() {
        let key = [11u8; KEY_LEN];
        let plaintext = b"same message";

        let a = aead_encrypt(&key, plaintext, &[]).unwrap();
        let b = aead_encrypt(&key, plaintext, &[]).unwrap();

        assert_ne!(a.nonce, b.nonce, "each encryption must use a fresh nonce");
        assert_ne!(
            a.ciphertext, b.ciphertext,
            "fresh nonce must yield distinct ciphertext for identical input"
        );
    }

    #[test]
    fn rng_helpers_have_correct_lengths() {
        assert_eq!(random_salt().len(), SALT_LEN);
        assert_eq!(random_nonce().len(), NONCE_LEN);
        assert_eq!(random_vek().len(), VEK_LEN);
    }

    #[test]
    fn rng_helpers_produce_distinct_values() {
        // Extremely unlikely to collide for a working CSPRNG.
        assert_ne!(random_salt(), random_salt());
        assert_ne!(random_nonce(), random_nonce());
        assert_ne!(*random_vek(), *random_vek());
    }

    #[test]
    fn random_below_stays_within_bounds() {
        for bound in [1u32, 2, 3, 7, 26, 64, 1000] {
            for _ in 0..1000 {
                let v = random_below(bound);
                assert!(v < bound, "random_below({bound}) returned {v}, out of range");
            }
        }
    }

    #[test]
    fn random_below_one_is_always_zero() {
        for _ in 0..100 {
            assert_eq!(random_below(1), 0);
        }
    }

    #[test]
    fn random_below_covers_full_range() {
        // Over enough draws every value in a small range should appear,
        // confirming the CSPRNG-backed selection is not stuck on a subset.
        let bound = 6u32;
        let mut seen = [false; 6];
        for _ in 0..5000 {
            seen[random_below(bound) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "every value in [0,{bound}) must occur");
    }

    #[test]
    fn random_below_is_roughly_uniform() {
        // A working CSPRNG with unbiased rejection sampling should distribute
        // draws fairly evenly; assert each bucket is within a generous tolerance
        // so the test is robust but still catches a badly skewed source.
        let bound = 10u32;
        let draws = 100_000u32;
        let mut counts = [0u32; 10];
        for _ in 0..draws {
            counts[random_below(bound) as usize] += 1;
        }
        let expected = draws / bound;
        for (i, &c) in counts.iter().enumerate() {
            let lo = expected / 2;
            let hi = expected + expected / 2;
            assert!(
                c >= lo && c <= hi,
                "bucket {i} count {c} outside tolerance [{lo}, {hi}]"
            );
        }
    }

    #[test]
    fn derived_key_round_trips_through_aead() {
        // End-to-end shape used by envelope encryption (task 2.2).
        let secret = b"master password";
        let salt = random_salt();
        let derived = derive_key(secret, &salt, fast_params()).unwrap();

        let vek = random_vek();
        let wrapped = aead_encrypt(&derived, vek.as_ref(), &[]).unwrap();
        let unwrapped = aead_decrypt(&derived, &wrapped, &[]).unwrap();

        assert_eq!(&unwrapped[..], vek.as_ref());
    }
}
