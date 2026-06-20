//! Versioned binary vault-file (de)serialization.
//!
//! This module owns the **on-disk wire format** of a Keyhaven vault file
//! (`.khv`): a plaintext, secret-free header followed by the encrypted payload.
//! It is a pure serialization layer — it performs **no cryptography**. It only
//! lays out the bytes of the header and the already-encrypted wraps/payload, and
//! parses them back, validating structure and version as it goes. The crypto
//! flows that produce the wraps and payload (create/unlock) live in task 3.3.
//!
//! ## File layout
//!
//! All multi-byte integers are encoded **little-endian**. Variable-length
//! fields (each wrap's ciphertext and the payload) are prefixed with a
//! `u32` length so decoding is unambiguous (Req 3.6).
//!
//! ```text
//! MAGIC         "KHVAULT\0"   (8 bytes)
//! FORMAT_VER    u16           (file format version, little-endian)
//! CIPHER_ID     u8            (1 = XChaCha20-Poly1305)
//! KDF_ID        u8            (1 = Argon2id)
//! KDF_PARAMS    m_cost: u32, t_cost: u32, p_cost: u32   (little-endian)
//! MASTER_SALT   16 bytes
//! ── password wrap ──
//! PW_NONCE      24 bytes
//! PW_CT_LEN     u32           (length of PW_CT)
//! PW_CT         PW_CT_LEN bytes   (VEK wrapped by the master key)
//! HAS_RECOVERY  u8            (0 = absent, 1 = present)
//! ── recovery wrap (only if HAS_RECOVERY == 1) ──
//! REC_SALT      16 bytes
//! REC_NONCE     24 bytes
//! REC_CT_LEN    u32
//! REC_CT        REC_CT_LEN bytes  (VEK wrapped by the recovery key)
//! ── payload ──
//! PAYLOAD_NONCE 24 bytes
//! PAYLOAD_LEN   u32
//! PAYLOAD       PAYLOAD_LEN bytes (AEAD ciphertext of the compressed model)
//! ```
//!
//! ## Tamper/format evidence (Req 15.4, 11.6)
//!
//! [`decode`] is total: it never panics on any input. It returns
//! [`VaultFormatError::VaultCorrupted`] for an unknown magic, a truncated or
//! structurally invalid file, or an unrecognized cipher/KDF identifier, and
//! [`VaultFormatError::IncompatibleVersion`] when `FORMAT_VER` is newer than
//! this build supports. This is the format-level half of tamper-evidence; the
//! AEAD authentication tags inside the wraps/payload (verified during the crypto
//! flows in task 3.3) catch bit-level tampering of the encrypted sections.
//! Import (Req 11.6) relies on this to validate a foreign file's magic/version
//! before any existing vault is touched.

// The format API is consumed by the create/open flows (task 3.3) and command
// layer (task 5); it is not yet referenced from the binary while the core is
// built bottom-up.
#![allow(dead_code)]

use crate::crypto::{
    AeadCiphertext, CIPHER_ID_XCHACHA20POLY1305, KDF_ID_ARGON2ID, KdfParams, NONCE_LEN, SALT_LEN,
};

/// File magic identifying a Keyhaven vault. Eight bytes so the header starts on
/// a recognizable, fixed boundary.
pub const MAGIC: &[u8; 8] = b"KHVAULT\0";

/// The highest file-format version this build can read and write. A file whose
/// `FORMAT_VER` exceeds this is rejected with
/// [`VaultFormatError::IncompatibleVersion`] rather than misread.
pub const FORMAT_VERSION: u16 = 1;

/// Errors produced while encoding or (more importantly) decoding a vault file.
///
/// These map onto the design's error model:
/// - [`VaultFormatError::VaultCorrupted`] → `KeyhavenError::VaultCorrupted`
/// - [`VaultFormatError::IncompatibleVersion`] → `KeyhavenError::IncompatibleVersion`
/// - [`VaultFormatError::Io`] → `KeyhavenError::Io`
#[derive(Debug)]
pub enum VaultFormatError {
    /// The bytes are not a structurally valid Keyhaven vault: bad/unknown magic,
    /// a truncated or oversized field, or an unrecognized cipher/KDF identifier.
    /// Deliberately coarse so it cannot be used as a parsing oracle.
    VaultCorrupted,
    /// The file's `FORMAT_VER` is newer than this build supports. Carries both
    /// values so the UI can prompt the user to update Keyhaven; the file must
    /// never be overwritten in this state.
    IncompatibleVersion {
        /// The version found in the file.
        found: u16,
        /// The highest version this build supports ([`FORMAT_VERSION`]).
        supported: u16,
    },
    /// An I/O error occurred while reading or writing the file stream. (The
    /// in-memory [`encode`]/[`decode`] paths do not produce this; it is provided
    /// for the stream-based callers in later tasks.)
    Io(std::io::Error),
}

impl PartialEq for VaultFormatError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (VaultFormatError::VaultCorrupted, VaultFormatError::VaultCorrupted) => true,
            (
                VaultFormatError::IncompatibleVersion {
                    found: a,
                    supported: b,
                },
                VaultFormatError::IncompatibleVersion {
                    found: c,
                    supported: d,
                },
            ) => a == c && b == d,
            // io::Error is not PartialEq; treat any two Io errors as unequal.
            _ => false,
        }
    }
}

impl std::fmt::Display for VaultFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultFormatError::VaultCorrupted => {
                write!(f, "this vault file is damaged or not a Keyhaven vault")
            }
            VaultFormatError::IncompatibleVersion { found, supported } => write!(
                f,
                "vault file format version {found} is newer than supported version {supported}; \
                 update Keyhaven to open it"
            ),
            VaultFormatError::Io(e) => write!(f, "vault file I/O error: {e}"),
        }
    }
}

impl std::error::Error for VaultFormatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VaultFormatError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for VaultFormatError {
    fn from(e: std::io::Error) -> Self {
        VaultFormatError::Io(e)
    }
}

/// A complete parsed vault file: the plaintext header plus the encrypted
/// payload bytes.
///
/// This carries everything the crypto layer (task 3.3) needs to derive keys and
/// decrypt — and nothing secret of its own. The `pw_wrap`, `recovery` wrap, and
/// `payload` are all already-encrypted blobs; this struct only moves their bytes
/// to and from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFile {
    /// File-format version. On [`encode`] this is always written as
    /// [`FORMAT_VERSION`]; on [`decode`] it reflects the file (and is guaranteed
    /// `<= FORMAT_VERSION`).
    pub format_ver: u16,
    /// Cipher identifier (currently only [`CIPHER_ID_XCHACHA20POLY1305`]).
    pub cipher_id: u8,
    /// KDF identifier (currently only [`KDF_ID_ARGON2ID`]).
    pub kdf_id: u8,
    /// Argon2id parameters used to derive the wrapping keys.
    pub kdf_params: KdfParams,
    /// Salt for the master-password KDF.
    pub master_salt: [u8; SALT_LEN],
    /// The VEK wrapped by the master-password-derived key.
    pub pw_wrap: AeadCiphertext,
    /// Optional recovery section: `(REC_SALT, REC_WRAP)`. `None` when the vault
    /// has no recovery key.
    pub recovery: Option<([u8; SALT_LEN], AeadCiphertext)>,
    /// Nonce used to encrypt the payload.
    pub payload_nonce: [u8; NONCE_LEN],
    /// AEAD ciphertext of the compressed JSON vault model.
    pub payload: Vec<u8>,
}

impl VaultFile {
    /// Build a [`VaultFile`] at the current [`FORMAT_VERSION`] with the standard
    /// cipher/KDF identifiers — the constructor used by the create flow once the
    /// wraps and encrypted payload have been produced.
    pub fn new(
        kdf_params: KdfParams,
        master_salt: [u8; SALT_LEN],
        pw_wrap: AeadCiphertext,
        recovery: Option<([u8; SALT_LEN], AeadCiphertext)>,
        payload_nonce: [u8; NONCE_LEN],
        payload: Vec<u8>,
    ) -> Self {
        VaultFile {
            format_ver: FORMAT_VERSION,
            cipher_id: CIPHER_ID_XCHACHA20POLY1305,
            kdf_id: KDF_ID_ARGON2ID,
            kdf_params,
            master_salt,
            pw_wrap,
            recovery,
            payload_nonce,
            payload,
        }
    }

    /// Serialize this vault file to its on-disk byte representation.
    ///
    /// The output round-trips through [`decode`] exactly (Property 3,
    /// format-level). Multi-byte integers are little-endian and variable-length
    /// blobs are length-prefixed as documented in the module header.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len_hint());

        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.format_ver.to_le_bytes());
        out.push(self.cipher_id);
        out.push(self.kdf_id);
        out.extend_from_slice(&self.kdf_params.m_cost.to_le_bytes());
        out.extend_from_slice(&self.kdf_params.t_cost.to_le_bytes());
        out.extend_from_slice(&self.kdf_params.p_cost.to_le_bytes());
        out.extend_from_slice(&self.master_salt);

        write_wrap(&mut out, &self.pw_wrap);

        match &self.recovery {
            Some((rec_salt, rec_wrap)) => {
                out.push(1);
                out.extend_from_slice(rec_salt);
                write_wrap(&mut out, rec_wrap);
            }
            None => out.push(0),
        }

        out.extend_from_slice(&self.payload_nonce);
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);

        out
    }

    fn encoded_len_hint(&self) -> usize {
        let rec_len = self
            .recovery
            .as_ref()
            .map(|(_, w)| SALT_LEN + NONCE_LEN + 4 + w.ciphertext.len())
            .unwrap_or(0);
        MAGIC.len()
            + 2 // format_ver
            + 1 // cipher_id
            + 1 // kdf_id
            + 12 // kdf params (3 * u32)
            + SALT_LEN
            + NONCE_LEN + 4 + self.pw_wrap.ciphertext.len()
            + 1 // has_recovery
            + rec_len
            + NONCE_LEN + 4 + self.payload.len()
    }
}

/// Parse a [`VaultFile`] from its on-disk byte representation.
///
/// This is **total** — it never panics, regardless of input. It returns:
/// - [`VaultFormatError::VaultCorrupted`] for an unknown magic, truncated or
///   trailing-garbage input, an implausibly large length prefix, or an
///   unrecognized cipher/KDF identifier;
/// - [`VaultFormatError::IncompatibleVersion`] when the file's `FORMAT_VER`
///   exceeds [`FORMAT_VERSION`].
pub fn decode(bytes: &[u8]) -> Result<VaultFile, VaultFormatError> {
    let mut r = Reader::new(bytes);

    // MAGIC — verified before anything else so foreign files fail fast.
    let magic = r.take(MAGIC.len())?;
    if magic != MAGIC {
        return Err(VaultFormatError::VaultCorrupted);
    }

    // FORMAT_VER — reject newer-than-supported before parsing the rest, so we
    // never misinterpret a layout we don't understand.
    let format_ver = r.u16()?;
    if format_ver > FORMAT_VERSION {
        return Err(VaultFormatError::IncompatibleVersion {
            found: format_ver,
            supported: FORMAT_VERSION,
        });
    }

    let cipher_id = r.u8()?;
    if cipher_id != CIPHER_ID_XCHACHA20POLY1305 {
        return Err(VaultFormatError::VaultCorrupted);
    }
    let kdf_id = r.u8()?;
    if kdf_id != KDF_ID_ARGON2ID {
        return Err(VaultFormatError::VaultCorrupted);
    }

    let kdf_params = KdfParams {
        m_cost: r.u32()?,
        t_cost: r.u32()?,
        p_cost: r.u32()?,
    };

    let master_salt = r.salt()?;
    let pw_wrap = r.wrap()?;

    let has_recovery = r.u8()?;
    let recovery = match has_recovery {
        0 => None,
        1 => {
            let rec_salt = r.salt()?;
            let rec_wrap = r.wrap()?;
            Some((rec_salt, rec_wrap))
        }
        // Any other value is structurally invalid.
        _ => return Err(VaultFormatError::VaultCorrupted),
    };

    let payload_nonce = r.nonce()?;
    let payload_len = r.u32()? as usize;
    let payload = r.take(payload_len)?.to_vec();

    // Reject trailing garbage: a valid file is fully consumed.
    if !r.is_empty() {
        return Err(VaultFormatError::VaultCorrupted);
    }

    Ok(VaultFile {
        format_ver,
        cipher_id,
        kdf_id,
        kdf_params,
        master_salt,
        pw_wrap,
        recovery,
        payload_nonce,
        payload,
    })
}

/// Append a length-prefixed wrap (`nonce` followed by `u32` length + ciphertext).
fn write_wrap(out: &mut Vec<u8>, wrap: &AeadCiphertext) {
    out.extend_from_slice(&wrap.nonce);
    out.extend_from_slice(&(wrap.ciphertext.len() as u32).to_le_bytes());
    out.extend_from_slice(&wrap.ciphertext);
}

/// A bounds-checked, panic-free cursor over the input bytes. Every read either
/// advances within bounds or returns [`VaultFormatError::VaultCorrupted`].
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Borrow exactly `n` bytes, advancing the cursor. Truncation is corruption.
    fn take(&mut self, n: usize) -> Result<&'a [u8], VaultFormatError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(VaultFormatError::VaultCorrupted)?;
        if end > self.buf.len() {
            return Err(VaultFormatError::VaultCorrupted);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, VaultFormatError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, VaultFormatError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, VaultFormatError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn salt(&mut self) -> Result<[u8; SALT_LEN], VaultFormatError> {
        let b = self.take(SALT_LEN)?;
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(b);
        Ok(salt)
    }

    fn nonce(&mut self) -> Result<[u8; NONCE_LEN], VaultFormatError> {
        let b = self.take(NONCE_LEN)?;
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(b);
        Ok(nonce)
    }

    /// Read a length-prefixed wrap. The `u32` length is validated against the
    /// remaining bytes by [`take`], so an oversized prefix yields corruption
    /// rather than an allocation or panic.
    fn wrap(&mut self) -> Result<AeadCiphertext, VaultFormatError> {
        let nonce = self.nonce()?;
        let len = self.u32()? as usize;
        let ciphertext = self.take(len)?.to_vec();
        Ok(AeadCiphertext { nonce, ciphertext })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wrap(seed: u8, ct_len: usize) -> AeadCiphertext {
        let mut nonce = [0u8; NONCE_LEN];
        for (i, b) in nonce.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        let ciphertext = (0..ct_len).map(|i| (i as u8) ^ seed).collect();
        AeadCiphertext { nonce, ciphertext }
    }

    fn sample_params() -> KdfParams {
        KdfParams {
            m_cost: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }

    fn file_without_recovery() -> VaultFile {
        VaultFile::new(
            sample_params(),
            [0xAB; SALT_LEN],
            sample_wrap(1, 48),
            None,
            [0xCD; NONCE_LEN],
            vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
        )
    }

    fn file_with_recovery() -> VaultFile {
        VaultFile::new(
            sample_params(),
            [0x11; SALT_LEN],
            sample_wrap(2, 48),
            Some(([0x22; SALT_LEN], sample_wrap(3, 48))),
            [0x33; NONCE_LEN],
            vec![100, 101, 102, 103, 104, 105],
        )
    }

    #[test]
    fn round_trip_without_recovery() {
        let file = file_without_recovery();
        let bytes = file.encode();
        let decoded = decode(&bytes).expect("valid file must decode");
        assert_eq!(decoded, file);
        assert!(decoded.recovery.is_none());
    }

    #[test]
    fn round_trip_with_recovery() {
        let file = file_with_recovery();
        let bytes = file.encode();
        let decoded = decode(&bytes).expect("valid file must decode");
        assert_eq!(decoded, file);
        let (rec_salt, rec_wrap) = decoded.recovery.expect("recovery section present");
        assert_eq!(rec_salt, [0x22; SALT_LEN]);
        assert_eq!(rec_wrap, sample_wrap(3, 48));
    }

    #[test]
    fn new_sets_current_version_and_ids() {
        let file = file_without_recovery();
        assert_eq!(file.format_ver, FORMAT_VERSION);
        assert_eq!(file.cipher_id, CIPHER_ID_XCHACHA20POLY1305);
        assert_eq!(file.kdf_id, KDF_ID_ARGON2ID);
    }

    #[test]
    fn header_begins_with_magic() {
        let bytes = file_without_recovery().encode();
        assert_eq!(&bytes[..MAGIC.len()], MAGIC);
    }

    #[test]
    fn variable_length_wraps_and_payload_round_trip() {
        // Exercise a range of sizes including zero-length payload/ciphertext.
        for &(pw_len, rec_len, payload_len) in &[
            (0usize, 0usize, 0usize),
            (1, 1, 1),
            (48, 48, 10),
            (200, 17, 4096),
            (32, 1024, 1),
        ] {
            let file = VaultFile::new(
                sample_params(),
                [0x5A; SALT_LEN],
                sample_wrap(7, pw_len),
                Some(([0x6B; SALT_LEN], sample_wrap(8, rec_len))),
                [0x7C; NONCE_LEN],
                (0..payload_len).map(|i| i as u8).collect(),
            );
            let decoded = decode(&file.encode()).expect("must round-trip");
            assert_eq!(decoded, file, "sizes pw={pw_len} rec={rec_len} pl={payload_len}");
        }
    }

    #[test]
    fn bad_magic_is_vault_corrupted() {
        let mut bytes = file_without_recovery().encode();
        bytes[0] ^= 0xFF; // corrupt the first magic byte
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);

        // Entirely foreign content.
        let foreign = b"not a keyhaven vault at all, just some bytes".to_vec();
        assert_eq!(
            decode(&foreign).unwrap_err(),
            VaultFormatError::VaultCorrupted
        );
    }

    #[test]
    fn empty_input_is_vault_corrupted() {
        assert_eq!(decode(&[]).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn newer_version_is_incompatible() {
        let mut bytes = file_without_recovery().encode();
        // Overwrite FORMAT_VER (2 LE bytes right after the 8-byte magic) with a
        // version greater than what we support.
        let newer = FORMAT_VERSION + 1;
        bytes[MAGIC.len()..MAGIC.len() + 2].copy_from_slice(&newer.to_le_bytes());

        assert_eq!(
            decode(&bytes).unwrap_err(),
            VaultFormatError::IncompatibleVersion {
                found: newer,
                supported: FORMAT_VERSION,
            }
        );
    }

    #[test]
    fn far_future_version_is_incompatible() {
        let mut bytes = file_without_recovery().encode();
        bytes[MAGIC.len()..MAGIC.len() + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            decode(&bytes).unwrap_err(),
            VaultFormatError::IncompatibleVersion {
                found: u16::MAX,
                supported: FORMAT_VERSION,
            }
        );
    }

    #[test]
    fn unknown_cipher_id_is_vault_corrupted() {
        let mut bytes = file_without_recovery().encode();
        // CIPHER_ID sits right after MAGIC (8) + FORMAT_VER (2).
        let cipher_idx = MAGIC.len() + 2;
        bytes[cipher_idx] = 0xEE;
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn unknown_kdf_id_is_vault_corrupted() {
        let mut bytes = file_without_recovery().encode();
        // KDF_ID sits right after MAGIC (8) + FORMAT_VER (2) + CIPHER_ID (1).
        let kdf_idx = MAGIC.len() + 2 + 1;
        bytes[kdf_idx] = 0xEE;
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn invalid_has_recovery_flag_is_vault_corrupted() {
        // Build a file without recovery, then flip HAS_RECOVERY to an illegal
        // value (neither 0 nor 1). It is located just after the password wrap.
        let file = file_without_recovery();
        let bytes = file.encode();
        let has_recovery_idx = MAGIC.len()
            + 2 // format_ver
            + 1 // cipher_id
            + 1 // kdf_id
            + 12 // kdf params
            + SALT_LEN
            + NONCE_LEN
            + 4
            + file.pw_wrap.ciphertext.len();
        let mut tampered = bytes.clone();
        tampered[has_recovery_idx] = 2;
        assert_eq!(
            decode(&tampered).unwrap_err(),
            VaultFormatError::VaultCorrupted
        );
    }

    #[test]
    fn truncation_at_every_length_never_panics() {
        // Decoding any prefix of a valid file must fail cleanly (never panic).
        let file = file_with_recovery();
        let bytes = file.encode();
        for cut in 0..bytes.len() {
            let err = decode(&bytes[..cut]).expect_err("a truncated file must not decode");
            // Truncation is always corruption (a truncated version field could
            // also be corruption, never a successful parse).
            assert_eq!(err, VaultFormatError::VaultCorrupted, "cut at {cut}");
        }
        // The full buffer still decodes.
        assert_eq!(decode(&bytes).unwrap(), file);
    }

    #[test]
    fn trailing_garbage_is_rejected() {
        let mut bytes = file_without_recovery().encode();
        bytes.push(0x00); // one extra byte beyond a complete file
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn oversized_payload_length_prefix_is_vault_corrupted() {
        // Encode a valid file, then rewrite the payload length prefix to a huge
        // value. The bounds-checked reader must reject it without allocating.
        let file = file_without_recovery();
        let mut bytes = file.encode();
        // PAYLOAD_LEN is the 4 bytes immediately before the payload.
        let payload_len_idx = bytes.len() - 4 - file.payload.len();
        bytes[payload_len_idx..payload_len_idx + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn oversized_wrap_length_prefix_is_vault_corrupted() {
        // Rewrite the password wrap's ciphertext length to an enormous value.
        let mut bytes = file_without_recovery().encode();
        // PW_CT_LEN starts after MAGIC + ver + cipher + kdf + params + salt + nonce.
        let pw_ct_len_idx =
            MAGIC.len() + 2 + 1 + 1 + 12 + SALT_LEN + NONCE_LEN;
        bytes[pw_ct_len_idx..pw_ct_len_idx + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes).unwrap_err(), VaultFormatError::VaultCorrupted);
    }

    #[test]
    fn kdf_params_survive_round_trip() {
        let params = KdfParams {
            m_cost: 19 * 1024,
            t_cost: 5,
            p_cost: 4,
        };
        let file = VaultFile::new(
            params,
            [0u8; SALT_LEN],
            sample_wrap(1, 48),
            None,
            [0u8; NONCE_LEN],
            vec![1, 2, 3],
        );
        let decoded = decode(&file.encode()).unwrap();
        assert_eq!(decoded.kdf_params, params);
    }

    #[test]
    fn version_field_is_little_endian() {
        // Confirm the documented little-endian encoding of FORMAT_VER.
        let bytes = file_without_recovery().encode();
        let ver = u16::from_le_bytes([bytes[MAGIC.len()], bytes[MAGIC.len() + 1]]);
        assert_eq!(ver, FORMAT_VERSION);
    }
}
