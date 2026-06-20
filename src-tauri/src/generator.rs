//! Password generator (task 8.1).
//!
//! Produces strong random passwords for new credentials (Req 10.x). The
//! generator is configurable in **length** and **character sets** (uppercase,
//! lowercase, digits, symbols) — Req 10.2 — and draws every character from the
//! OS CSPRNG via the crypto module's [`crypto::random_below`] helper, so it
//! reuses the application's single audited randomness source rather than
//! introducing a new RNG (Req 10.3).
//!
//! ## Generation strategy (Correctness Property 11)
//!
//! Given the selected character sets, [`generate_password`]:
//! 1. Places **one** character from each selected set first, guaranteeing the
//!    result actually contains every class the user asked for (so a password
//!    requested with digits is never returned without a digit).
//! 2. Fills the remaining positions from the combined pool of selected sets.
//! 3. Shuffles the whole buffer with a CSPRNG-backed Fisher-Yates pass so the
//!    guaranteed characters are not predictably positioned at the front.
//!
//! The output therefore satisfies the requested length and draws **only** from
//! the selected sets (Property 11). The working buffer is held in [`Zeroizing`]
//! so the generated material is best-effort cleared from memory once returned.

// The generator command is wired into the IPC surface by task 8.x; while the
// backend is built bottom-up some items are not yet referenced from the binary.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto;
use crate::error::KeyhavenError;
use crate::model::PasswordGenDefaults;

/// Uppercase letters (no ambiguity stripping — full set).
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// Lowercase letters.
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
/// Decimal digits.
const DIGITS: &[u8] = b"0123456789";
/// Symbol set — a broad, widely-accepted punctuation selection.
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.?/";

/// Smallest password length the generator will produce.
pub const MIN_LENGTH: u32 = 1;
/// Largest password length the generator will produce. A generous upper bound
/// that guards against accidental/abusive huge allocations.
pub const MAX_LENGTH: u32 = 512;

/// Options controlling password generation (Req 10.2).
///
/// Mirrors the design's `PasswordGenOptions { length, upper, lower, digits,
/// symbols }` and the stored [`PasswordGenDefaults`]. Serializes in camelCase to
/// match the rest of the IPC surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordGenOptions {
    /// Desired password length (characters). Must be within
    /// `[MIN_LENGTH, MAX_LENGTH]` and at least the number of selected sets.
    pub length: u32,
    /// Include uppercase letters.
    pub upper: bool,
    /// Include lowercase letters.
    pub lower: bool,
    /// Include digits.
    pub digits: bool,
    /// Include symbols.
    pub symbols: bool,
}

impl Default for PasswordGenOptions {
    fn default() -> Self {
        PasswordGenOptions::from(PasswordGenDefaults::default())
    }
}

impl From<PasswordGenDefaults> for PasswordGenOptions {
    fn from(d: PasswordGenDefaults) -> Self {
        PasswordGenOptions {
            length: d.length,
            upper: d.upper,
            lower: d.lower,
            digits: d.digits,
            symbols: d.symbols,
        }
    }
}

impl PasswordGenOptions {
    /// Collect the selected character sets in a stable order.
    fn selected_sets(&self) -> Vec<&'static [u8]> {
        let mut sets: Vec<&'static [u8]> = Vec::with_capacity(4);
        if self.upper {
            sets.push(UPPER);
        }
        if self.lower {
            sets.push(LOWER);
        }
        if self.digits {
            sets.push(DIGITS);
        }
        if self.symbols {
            sets.push(SYMBOLS);
        }
        sets
    }
}

/// Generate a strong random password from `opts` (Req 10.1, 10.2, 10.3).
///
/// Returns [`KeyhavenError::InvalidInput`] when the options cannot produce a
/// valid password:
/// - no character set is selected,
/// - `length` is outside `[MIN_LENGTH, MAX_LENGTH]`, or
/// - `length` is smaller than the number of selected sets (so at-least-one of
///   each requested class cannot be guaranteed).
///
/// On success the returned password has exactly `opts.length` characters, drawn
/// only from the selected sets, with at least one character from each selected
/// set, using the CSPRNG for every choice.
pub fn generate_password(opts: PasswordGenOptions) -> Result<String, KeyhavenError> {
    let sets = opts.selected_sets();
    if sets.is_empty() {
        return Err(KeyhavenError::InvalidInput {
            message: "select at least one character set".to_string(),
        });
    }
    if opts.length < MIN_LENGTH || opts.length > MAX_LENGTH {
        return Err(KeyhavenError::InvalidInput {
            message: format!("length must be between {MIN_LENGTH} and {MAX_LENGTH}"),
        });
    }
    if (opts.length as usize) < sets.len() {
        return Err(KeyhavenError::InvalidInput {
            message: format!(
                "length must be at least {} to include one of each selected set",
                sets.len()
            ),
        });
    }

    let length = opts.length as usize;
    // Combined pool of all characters from the selected sets.
    let pool: Vec<u8> = sets.iter().flat_map(|s| s.iter().copied()).collect();

    let mut chars = Zeroizing::new(Vec::<u8>::with_capacity(length));

    // 1. Guarantee at least one character from each selected set (Property 11).
    for set in &sets {
        chars.push(pick(set));
    }
    // 2. Fill the rest from the combined pool.
    while chars.len() < length {
        chars.push(pick(&pool));
    }
    // 3. CSPRNG-backed Fisher-Yates shuffle so the guaranteed characters are not
    //    predictably positioned at the front.
    for i in (1..chars.len()).rev() {
        let j = crypto::random_below((i + 1) as u32) as usize;
        chars.swap(i, j);
    }

    // All characters are ASCII from the constant sets above, so this is valid
    // UTF-8 by construction.
    Ok(String::from_utf8(chars.to_vec()).expect("charset bytes are valid ASCII/UTF-8"))
}

/// Pick a single uniformly-random byte from `set` using the CSPRNG. `set` is
/// always non-empty at every call site.
fn pick(set: &[u8]) -> u8 {
    set[crypto::random_below(set.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn opts(length: u32, upper: bool, lower: bool, digits: bool, symbols: bool) -> PasswordGenOptions {
        PasswordGenOptions {
            length,
            upper,
            lower,
            digits,
            symbols,
        }
    }

    fn allowed_bytes(o: &PasswordGenOptions) -> HashSet<u8> {
        o.selected_sets()
            .iter()
            .flat_map(|s| s.iter().copied())
            .collect()
    }

    #[test]
    fn respects_requested_length() {
        // Single charset so even length 1 is valid (no min-set-count floor).
        for len in [1u32, 4, 8, 20, 64, 128, MAX_LENGTH] {
            let pw = generate_password(opts(len, false, true, false, false)).unwrap();
            assert_eq!(pw.chars().count(), len as usize, "length {len} not honored");
        }
        // And with all sets enabled for lengths that can fit one of each.
        for len in [4u32, 8, 20, 64, MAX_LENGTH] {
            let pw = generate_password(opts(len, true, true, true, true)).unwrap();
            assert_eq!(pw.chars().count(), len as usize, "length {len} not honored");
        }
    }

    #[test]
    fn only_uses_selected_charsets() {
        let o = opts(40, true, false, true, false); // upper + digits only
        let allowed = allowed_bytes(&o);
        let pw = generate_password(o).unwrap();
        for b in pw.bytes() {
            assert!(allowed.contains(&b), "byte {b:?} outside selected sets");
        }
        // And no lowercase/symbol leaked in.
        assert!(pw.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
    }

    #[test]
    fn includes_at_least_one_of_each_selected_set() {
        // A long password with all sets should, deterministically, contain at
        // least one character from every selected class (guaranteed by step 1).
        let pw = generate_password(opts(40, true, true, true, true)).unwrap();
        assert!(pw.bytes().any(|b| UPPER.contains(&b)), "missing uppercase");
        assert!(pw.bytes().any(|b| LOWER.contains(&b)), "missing lowercase");
        assert!(pw.bytes().any(|b| DIGITS.contains(&b)), "missing digit");
        assert!(pw.bytes().any(|b| SYMBOLS.contains(&b)), "missing symbol");
    }

    #[test]
    fn single_charset_uses_only_that_set() {
        let pw = generate_password(opts(30, false, true, false, false)).unwrap();
        assert!(pw.bytes().all(|b| b.is_ascii_lowercase()));
        assert_eq!(pw.len(), 30);
    }

    #[test]
    fn minimum_length_with_all_sets_has_one_of_each() {
        // length == number of selected sets: every position must be a distinct
        // class, so exactly one of each.
        let pw = generate_password(opts(4, true, true, true, true)).unwrap();
        assert_eq!(pw.len(), 4);
        assert_eq!(pw.bytes().filter(|b| UPPER.contains(b)).count(), 1);
        assert_eq!(pw.bytes().filter(|b| LOWER.contains(b)).count(), 1);
        assert_eq!(pw.bytes().filter(|b| DIGITS.contains(b)).count(), 1);
        assert_eq!(pw.bytes().filter(|b| SYMBOLS.contains(b)).count(), 1);
    }

    #[test]
    fn rejects_no_charset_selected() {
        let err = generate_password(opts(16, false, false, false, false)).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn rejects_zero_length() {
        let err = generate_password(opts(0, true, true, true, true)).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn rejects_length_below_selected_set_count() {
        // 3 chars cannot include one of each of the 4 selected sets.
        let err = generate_password(opts(3, true, true, true, true)).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn rejects_length_above_maximum() {
        let err = generate_password(opts(MAX_LENGTH + 1, true, true, true, true)).unwrap_err();
        assert!(matches!(err, KeyhavenError::InvalidInput { .. }));
    }

    #[test]
    fn defaults_match_stored_password_gen_defaults() {
        let o = PasswordGenOptions::default();
        let d = PasswordGenDefaults::default();
        assert_eq!(o.length, d.length);
        assert_eq!(o.upper, d.upper);
        assert_eq!(o.lower, d.lower);
        assert_eq!(o.digits, d.digits);
        assert_eq!(o.symbols, d.symbols);
        // The default options generate successfully.
        assert_eq!(generate_password(o).unwrap().len(), d.length as usize);
    }

    #[test]
    fn successive_passwords_differ() {
        // With a CSPRNG source, two 24-char passwords colliding is astronomically
        // unlikely; a fixed/empty RNG would make them equal.
        let o = opts(24, true, true, true, true);
        let a = generate_password(o).unwrap();
        let b = generate_password(o).unwrap();
        assert_ne!(a, b, "generated passwords must vary across calls (CSPRNG)");
    }

    /// Correctness Property 11 (Generator compliance): across many randomized
    /// option combinations, every generated password satisfies the requested
    /// length and draws only from the selected character sets, with at least one
    /// character of each selected set.
    ///
    /// **Validates: Requirements 10.2, 10.3**
    #[test]
    fn property_generator_compliance_over_many_options() {
        for _ in 0..2000 {
            // Randomly choose options using the same CSPRNG helper under test.
            let upper = crypto::random_below(2) == 1;
            let lower = crypto::random_below(2) == 1;
            let digits = crypto::random_below(2) == 1;
            let symbols = crypto::random_below(2) == 1;
            if !(upper || lower || digits || symbols) {
                continue; // skip the empty selection (separately tested)
            }
            let selected = [upper, lower, digits, symbols]
                .iter()
                .filter(|&&b| b)
                .count() as u32;
            // length in [selected, selected + 40]
            let length = selected + crypto::random_below(41);

            let o = opts(length, upper, lower, digits, symbols);
            let pw = generate_password(o).expect("valid options must generate");
            let allowed = allowed_bytes(&o);

            assert_eq!(pw.chars().count(), length as usize, "length mismatch");
            for b in pw.bytes() {
                assert!(allowed.contains(&b), "char outside selected sets");
            }
            if upper {
                assert!(pw.bytes().any(|b| UPPER.contains(&b)), "missing required uppercase");
            }
            if lower {
                assert!(pw.bytes().any(|b| LOWER.contains(&b)), "missing required lowercase");
            }
            if digits {
                assert!(pw.bytes().any(|b| DIGITS.contains(&b)), "missing required digit");
            }
            if symbols {
                assert!(pw.bytes().any(|b| SYMBOLS.contains(&b)), "missing required symbol");
            }
        }
    }
}
