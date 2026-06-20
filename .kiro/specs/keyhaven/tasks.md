# Implementation Plan

## Overview

This plan builds Keyhaven bottom-up: the security-critical Rust core (crypto → vault format → session → commands) is implemented and tested first, then the Svelte UI is layered on top, and finally everything is integrated and packaged cross-platform. Backend tasks carry the heaviest test coverage because they hold the trust boundary. Frontend tasks assume the corresponding backend commands already exist.

## Tasks

- [x] 1. Scaffold the Tauri 2 + Svelte project
  - Initialize a Tauri 2 app with a Svelte + Vite (SPA mode) frontend in the workspace root.
  - Configure Tauri capabilities/allowlist to disable all networking and enable only filesystem (dialog-scoped) and clipboard permissions.
  - Verify the app builds and launches with an empty placeholder window on the current OS.
  - Set up the Rust module skeleton (`crypto/`, `vault/`, `session/`, `catalog/`, `commands.rs`, `model.rs`) and the Svelte folder skeleton (`lib/api.ts`, `lib/stores/`, `lib/components/`, `lib/design/`, views).
  - _Requirements: 13.1, 13.2, 13.3_

- [x] 2. Implement the cryptographic core (Rust)
- [x] 2.1 Implement KDF and AEAD primitives with audited crates
  - Add `argon2`, `chacha20poly1305`, `rand`/`getrandom`, and `zeroize` dependencies.
  - Implement `derive_key(secret, salt, params) -> Zeroizing<[u8;32]>` using Argon2id.
  - Implement `aead_encrypt`/`aead_decrypt` (XChaCha20-Poly1305) with random nonce generation.
  - Implement CSPRNG helpers for salts, nonces, and the VEK.
  - Write unit tests: derivation determinism, encrypt/decrypt round-trip, wrong-key rejection, and tamper detection (flip a ciphertext byte → auth failure).
  - _Requirements: 15.1, 15.2, 15.6_

- [x] 2.2 Implement envelope encryption (VEK wrapping/unwrapping)
  - Implement generating a random VEK and wrapping it with a derived key (password or recovery).
  - Implement unwrapping the VEK and surfacing a `WrongCredentials` error on auth-tag failure.
  - Wrap all keys/secrets in `Zeroizing` and ensure they are cleared after use.
  - Write unit tests for wrap/unwrap with correct and incorrect keys.
  - _Requirements: 2.3, 2.8, 15.3, 15.5_

- [x] 3. Implement the vault file format and repository (Rust)
- [x] 3.1 Define data models and (de)serialization
  - Implement `model.rs` structs (vault model, entry, field, custom service, settings) with serde.
  - Implement JSON serialization + compression of the decrypted vault model.
  - _Requirements: 5.4, 7.6, 12.5_

- [x] 3.2 Implement the versioned binary header read/write
  - Implement encoding/decoding of MAGIC, FORMAT_VER, CIPHER_ID, KDF_ID, KDF params, salts, wraps, and payload nonce.
  - Reject unknown magic (`VaultCorrupted`) and newer-than-supported versions (`IncompatibleVersion`).
  - Write unit tests for header round-trip and rejection cases.
  - _Requirements: 3.6, 11.6, 15.4_

- [x] 3.3 Implement vault create/open and atomic, crash-safe writes
  - Implement create-vault flow (VEK + salts + wraps + encrypted payload) per the design crypto flow.
  - Implement open/unlock flows for both password and recovery-key paths.
  - Implement atomic write (temp file + fsync + rename) and retain a `.bak` of the prior version.
  - Write tests: full create→write→read→unlock round-trip (password and recovery), and mid-write failure recovery.
  - _Requirements: 1.5, 1.6, 1.7, 1.8, 2.7, 3.2, 3.6_

- [x] 4. Implement the session manager and auto-lock (Rust)
- [x] 4.1 Implement in-memory unlocked session state
  - Hold the VEK and decrypted model in session memory only while unlocked; expose `is_unlocked`.
  - Implement `lock_vault` to drop and zeroize the model and VEK and emit a `vault-locked` event.
  - Gate entry commands so they return `Locked` when the vault is locked.
  - Write tests for lock/unlock state transitions and post-lock zeroization behavior.
  - _Requirements: 3.4, 3.5, 15.5_

- [x] 4.2 Implement the backend auto-lock timer
  - Implement an inactivity deadline reset on reported activity and a timer task that locks on expiry.
  - Implement optional lock-on-blur via the Tauri window blur/minimize event.
  - Make timeout and lock-on-blur configurable via settings.
  - Write tests for deadline reset and expiry-triggered lock.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 5. Implement master password and recovery commands (Rust)
- [x] 5.1 Implement create_vault and recovery-key generation
  - Implement `create_vault` returning the recovery key exactly once when requested.
  - Implement password-only creation path when recovery is declined.
  - _Requirements: 1.1, 1.2, 1.3, 2.1, 2.2, 2.6_

- [x] 5.2 Implement unlock, recovery-unlock, and change-password commands
  - Implement `unlock_with_password`, `unlock_with_recovery_key`, and `change_master_password` (rewriting only the password wrap + salt).
  - Verify recovery-unlock prompts/permits setting a new master password and that recovery still works afterward.
  - Write tests for change-password (payload unchanged, old rejected, new accepted, recovery intact).
  - _Requirements: 2.7, 3.1, 3.2, 3.3_

- [x] 6. Implement the entry repository and CRUD commands (Rust)
- [x] 6.1 Implement entry create/read/update/delete over the decrypted model
  - Implement CRUD with UUIDs and created/updated timestamps, persisting via the atomic write path.
  - Implement flexible fields (add/remove/custom label, secret flag) and empty-entry warning signal.
  - Write tests for CRUD correctness and timestamp updates.
  - _Requirements: 5.3, 5.4, 5.5, 5.6, 7.2, 7.3, 7.5, 7.6, 8.5, 8.6_

- [x] 6.2 Implement search index and paginated listing
  - Build an in-memory index over non-secret fields only; implement filtered, paginated `list_entries` returning lightweight summaries.
  - Write tests confirming secret values are never indexed and that pagination/filtering is correct.
  - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5_

- [x] 7. Implement the service catalog (Rust + dev tooling)
- [x] 7.1 Build the dev-time catalog generation script
  - Implement `tools/build-catalog.mjs` (Node.js) to assemble a curated `services-catalog.json` of ~50 popular global services with names, aliases, icons, and recommended fields.
  - Document that this script is dev-only and not shipped; only its JSON/icon output is bundled.
  - _Requirements: 12.1, 12.5_

- [x] 7.2 Implement catalog loading and custom services
  - Load the bundled catalog at startup; implement `search_catalog`.
  - Implement `create_custom_service`/`list_custom_services` persisted in the vault, with a "Custom" indicator flag and custom icon support.
  - Write tests for catalog search and custom-service persistence.
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 12.2, 12.3, 12.4_

- [x] 8. Implement utility commands: generator, clipboard, settings, export/import (Rust)
- [x] 8.1 Implement the password generator
  - Implement `generate_password` with configurable length/charsets using the CSPRNG.
  - Write tests asserting option compliance and randomness source.
  - _Requirements: 10.1, 10.2, 10.3, 10.4_

- [x] 8.2 Implement clipboard copy with auto-clear
  - Implement `copy_secret_to_clipboard` and a scheduled clear that only clears if the value is unchanged.
  - _Requirements: 8.3, 8.4_

- [x] 8.3 Implement settings persistence and export/import
  - Implement `get_settings`/`update_settings` (stored in the encrypted vault).
  - Implement `export_vault` (encrypted single file to chosen path) and `import_vault` (validate magic/version before touching existing vault).
  - Write tests for import validation rejecting malformed/foreign files without altering the current vault.
  - _Requirements: 4.3, 8.4, 11.1, 11.2, 11.3, 11.4, 11.6_

- [x] 9. Build the frontend design system and shell (Svelte)
- [x] 9.1 Implement design tokens and global styles
  - Implement `lib/design/tokens.css` (soft off-white backgrounds, calm blue accent, muted status colors, radii, soft shadows, spacing scale, system font stack).
  - Implement base layout, typography, and `prefers-reduced-motion` handling.
  - _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5_

- [x] 9.2 Implement the API wrapper, stores, and routing shell
  - Implement `lib/api.ts` over Tauri `invoke`, the Svelte stores (session, entries, searchQuery, settings, toast), and route gating based on session status.
  - Listen for the `vault-locked` event to route back to Unlock.
  - _Requirements: 3.1, 3.4, 4.2_

- [x] 10. Implement setup, unlock, and recovery screens (Svelte)
- [x] 10.1 Implement the Setup (first-run) screen
  - Implement master password creation with confirm field, strength meter, and mismatch error.
  - Implement optional recovery-key generation with one-time reveal, "I saved it" confirmation gate, and the security warning; support the decline path with its warning.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.4, 2.5, 2.6_

- [x] 10.2 Implement the Unlock and Recovery screens
  - Implement the unlock screen with password entry and error handling, plus a "use recovery key instead" path.
  - Implement the recovery screen: enter recovery key → set new master password.
  - _Requirements: 2.7, 3.1, 3.2, 3.3_

- [x] 11. Implement the main vault view with search at scale (Svelte)
  - Implement the entry list grouped/identifiable by service icon with a prominent Add button and manual Lock control.
  - Implement debounced search wired to `list_entries` and a virtualized list for smooth rendering of thousands of entries.
  - _Requirements: 8.1, 9.1, 9.2, 9.3, 3.5_

- [x] 12. Implement the entry editor (Svelte)
- [x] 12.1 Implement the service picker and field prefill
  - Implement a searchable catalog picker plus a "Create custom service" flow (name + icon, Custom badge).
  - Prefill recommended fields on service selection.
  - _Requirements: 5.1, 5.2, 6.1, 6.2, 6.3, 7.1, 12.2_

- [x] 12.2 Implement field editing, masking, reveal/copy, and inline generator
  - Implement add/remove/custom-label fields, secret masking with reveal, copy-to-clipboard (auto-clear), and an inline password generator.
  - Implement save/validation (empty-entry warning) and delete with confirmation.
  - _Requirements: 5.3, 5.6, 7.2, 7.3, 7.4, 7.5, 8.2, 8.3, 8.6, 10.1_

- [x] 13. Implement the settings screen and backup guidance (Svelte)
  - Implement controls for auto-lock timeout (incl. disable), lock-on-blur, clipboard-clear delay, and password-generator defaults.
  - Implement export/import UI with file dialogs and prominent "keep backups in multiple safe places" guidance.
  - _Requirements: 4.3, 8.4, 11.1, 11.3, 11.5, 14.5_

- [x] 14. Frontend component and store tests (Svelte)
  - Add Vitest + Testing Library tests for EntryEditor (field add/remove, mask/reveal), service picker, password generator UI, and store logic (debounced search, session transitions).
  - _Requirements: 7.2, 7.3, 7.4, 9.2, 10.2_

- [x] 15. Integration, cross-platform packaging, and final verification
  - Add Tauri integration smoke tests (create/unlock/CRUD/lock against a temp vault).
  - Configure cross-platform build/bundle for Windows, macOS, and Linux and verify launch on the current platform.
  - Verify no-network behavior, accessibility contrast (AA target), and reduced-motion handling; clean up any temporary artifacts.
  - _Requirements: 9.5, 13.1, 13.2, 13.3, 13.4, 14.4_

## Task Dependency Graph

```
1 (scaffold)
├── 2 (crypto core)
│   ├── 2.1 → 2.2
│   └── 3 (vault format & repository)
│       ├── 3.1 → 3.2 → 3.3
│       ├── 4 (session & auto-lock)
│       │   └── 4.1 → 4.2
│       ├── 5 (master pw & recovery commands)
│       │   └── 5.1 → 5.2
│       ├── 6 (entry repository & CRUD)
│       │   └── 6.1 → 6.2
│       ├── 7 (service catalog)
│       │   └── 7.1 → 7.2
│       └── 8 (utilities: generator, clipboard, settings, export/import)
│           ├── 8.1
│           ├── 8.2
│           └── 8.3
└── 9 (frontend design system & shell)        [depends on 1; consumes commands from 4–8]
    ├── 9.1 → 9.2
    ├── 10 (setup/unlock/recovery screens)     [needs 5]
    │   └── 10.1 → 10.2
    ├── 11 (main vault view + search)          [needs 6]
    ├── 12 (entry editor)                      [needs 6, 7, 8.1, 8.2]
    │   └── 12.1 → 12.2
    └── 13 (settings + backup guidance)        [needs 8.3, 4.2]

14 (frontend tests)        [depends on 10–13]
15 (integration & packaging)   [depends on all]
```

Build order summary: 1 → 2 → 3 → (4, 5, 6, 7, 8 in any order) → 9 → (10, 11, 12, 13) → 14 → 15.

```json
{
  "waves": [
    { "wave": 1, "tasks": ["1"], "dependsOn": [] },
    { "wave": 2, "tasks": ["2.1"], "dependsOn": ["1"] },
    { "wave": 3, "tasks": ["2.2"], "dependsOn": ["2.1"] },
    { "wave": 4, "tasks": ["3.1"], "dependsOn": ["2.2"] },
    { "wave": 5, "tasks": ["3.2"], "dependsOn": ["3.1"] },
    { "wave": 6, "tasks": ["3.3"], "dependsOn": ["3.2"] },
    { "wave": 7, "tasks": ["4.1", "5.1", "6.1", "7.1", "8.1", "8.2"], "dependsOn": ["3.3"] },
    { "wave": 8, "tasks": ["4.2", "5.2", "6.2", "7.2", "8.3"], "dependsOn": ["4.1", "5.1", "6.1", "7.1"] },
    { "wave": 9, "tasks": ["9.1"], "dependsOn": ["1"] },
    { "wave": 10, "tasks": ["9.2"], "dependsOn": ["9.1", "4.2", "5.2", "6.2", "7.2", "8.3"] },
    { "wave": 11, "tasks": ["10.1", "11", "12.1", "13"], "dependsOn": ["9.2"] },
    { "wave": 12, "tasks": ["10.2", "12.2"], "dependsOn": ["10.1", "12.1"] },
    { "wave": 13, "tasks": ["14"], "dependsOn": ["10.2", "11", "12.2", "13"] },
    { "wave": 14, "tasks": ["15"], "dependsOn": ["14"] }
  ]
}
```

## Notes

- Per project guidance, automated tests are written as part of each implementing task rather than as a separate retrofit phase. Test expectations are embedded in the relevant sub-tasks above.
- The dev-time catalog script (Task 7.1) is tooling only and is never shipped to users; only its generated JSON/icon output is bundled.
- All cryptographic work stays in the Rust backend (Tasks 2–8); the Svelte layer (Tasks 9–13) never performs encryption and only holds decrypted data transiently.
- Each task references specific acceptance criteria from `requirements.md` so coverage is traceable.
