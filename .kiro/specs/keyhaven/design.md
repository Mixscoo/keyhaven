# Design Document

## Overview

Keyhaven is an offline-first, cross-platform desktop password manager built with **Tauri 2** (Rust backend) and **Svelte** (frontend). All credential data lives in a single, portable, strongly-encrypted vault file. The application performs zero network communication: there is no telemetry, no sync, and no cloud dependency.

The design enforces a strict **trust boundary**: all cryptographic operations and secret handling occur in the Rust backend, where memory safety and `zeroize`-based secret wiping are available. The Svelte frontend is treated as a "dumb" presentation layer that never performs encryption and only holds decrypted data transiently in memory while the vault is unlocked.

### Design Goals

1. **Zero-knowledge**: Without the master password or recovery key, the vault file is cryptographically unreadable.
2. **Offline & private**: No network access, ever. No data leaves the machine.
3. **Portable**: One self-contained file, openable on any device running Keyhaven.
4. **Performant at scale**: Smooth with thousands of entries.
5. **Calm & accessible**: Minimalist light theme, soft blue accent, gentle motion.
6. **Cross-platform**: Windows, macOS, Linux from one codebase.

### Key Technical Decisions

| Concern | Decision | Rationale |
|---|---|---|
| Desktop shell | Tauri 2 | Tiny binaries, low memory, Rust backend, native webviews |
| Frontend | Svelte + Vite (SPA mode) | Lean output, built-in transitions, low boilerplate |
| KDF | Argon2id | Current best-practice memory-hard KDF, resists GPU/ASIC brute force |
| Cipher | XChaCha20-Poly1305 (primary) | Authenticated; large random nonce avoids nonce-reuse pitfalls; constant-time in software |
| Crypto location | Rust backend only | Memory safety, zeroization, audited crates |
| Vault format | Versioned binary header + encrypted JSON payload | Future-proof, integrity-protected, single file |
| Secret wiping | `zeroize` crate | Best-effort clearing of keys/secrets from memory |

> Note on cipher: XChaCha20-Poly1305 is chosen as the primary cipher because its 192-bit random nonce removes the need for careful nonce counter management (a common AES-GCM footgun). AES-256-GCM remains a documented alternative; the vault header records which cipher was used so future versions can support either.

## Architecture

### High-Level Structure

```
┌───────────────────────────────────────────────────────────┐
│                     Keyhaven (Tauri App)                    │
│                                                             │
│  ┌─────────────────────────┐      ┌──────────────────────┐ │
│  │   Svelte Frontend (UI)  │      │   Rust Backend (Core) │ │
│  │                         │      │                       │ │
│  │  - Unlock / Setup       │ IPC  │  - Crypto service     │ │
│  │  - Entry list & search  │◄────►│  - Vault repository   │ │
│  │  - Entry editor         │ cmds │  - Session manager    │ │
│  │  - Settings             │      │  - Catalog provider   │ │
│  │  - Password generator   │      │  - Auto-lock timer    │ │
│  │  - Svelte stores (state)│      │  - Secret zeroization │ │
│  └─────────────────────────┘      └──────────────────────┘ │
│           ▲                                  │              │
│           │ decrypted data (in-memory only)  ▼              │
│           │                          ┌──────────────────┐   │
│           │                          │  vault file (.khv)│  │
│           └──────────────────────────│  on local disk    │  │
│                                       └──────────────────┘  │
└───────────────────────────────────────────────────────────┘
```

### Trust Boundary

- **Rust backend (trusted)**: holds the derived encryption key and decrypted vault in memory only while unlocked; performs all encrypt/decrypt; reads/writes the vault file; manages the auto-lock timer; zeroizes secrets on lock/exit.
- **Svelte frontend (presentation)**: requests data via Tauri commands; renders decrypted entries; never sees the master password after it is passed once to the backend for unlocking; never sees the derived key.

The master password and recovery key are passed from frontend to backend over Tauri's IPC exactly when needed (setup, unlock, recovery, change-password) and are zeroized immediately after key derivation. They are never persisted and never echoed back.

### Process / Module Layout

**Rust backend (`src-tauri/src/`)**
- `crypto/` — KDF (Argon2id), AEAD (XChaCha20-Poly1305), random generation, key wrapping.
- `vault/` — vault file (de)serialization, versioned header, repository (CRUD over decrypted model).
- `session/` — in-memory unlocked state, derived key handling, auto-lock timer, lock/zeroize.
- `catalog/` — loads bundled service catalog, provides search and recommended fields.
- `commands.rs` — Tauri command handlers (the IPC surface).
- `model.rs` — serde data structures shared across modules.

**Svelte frontend (`src/`)**
- `routes/` or `views/` — Setup, Unlock, Recovery, Vault (list), EntryEditor, Settings.
- `lib/api.ts` — thin wrappers over Tauri `invoke` commands.
- `lib/stores/` — Svelte stores for session state, entries, search query, settings.
- `lib/components/` — reusable UI (EntryCard, FieldRow, ServicePicker, PasswordGenerator, Modal, Toast).
- `lib/design/` — design tokens (colors, spacing, typography), global CSS.

## Components and Interfaces

### Tauri Command Surface (IPC API)

All commands are async and return a typed `Result<T, KeyhavenError>`. No command ever returns the master password, recovery key, or derived key.

```rust
// ---- Vault lifecycle ----
fn vault_exists(path: Option<String>) -> Result<bool>;
fn create_vault(master_password: String, generate_recovery: bool, path: String)
    -> Result<CreateVaultResult>; // returns recovery_key ONCE if generated
fn unlock_with_password(master_password: String, path: String) -> Result<VaultSummary>;
fn unlock_with_recovery_key(recovery_key: String, path: String) -> Result<VaultSummary>;
fn change_master_password(current: String, new_password: String) -> Result<()>;
fn lock_vault() -> Result<()>;
fn is_unlocked() -> Result<bool>;

// ---- Entries (require unlocked session) ----
fn list_entries(query: Option<String>, page: Option<Page>) -> Result<EntryList>;
fn get_entry(id: String) -> Result<Entry>;
fn create_entry(input: EntryInput) -> Result<Entry>;
fn update_entry(id: String, input: EntryInput) -> Result<Entry>;
fn delete_entry(id: String) -> Result<()>;

// ---- Services / catalog ----
fn search_catalog(query: String) -> Result<Vec<CatalogService>>;
fn list_custom_services() -> Result<Vec<CustomService>>;
fn create_custom_service(name: String, icon: IconRef) -> Result<CustomService>;

// ---- Utilities ----
fn generate_password(opts: PasswordGenOptions) -> Result<String>;
fn copy_secret_to_clipboard(entry_id: String, field_id: String) -> Result<()>; // schedules auto-clear
fn export_vault(destination: String) -> Result<()>;
fn import_vault(source: String) -> Result<()>;

// ---- Settings ----
fn get_settings() -> Result<Settings>;
fn update_settings(settings: Settings) -> Result<()>;
```

### Frontend State (Svelte stores)

- `session` — `{ status: 'no-vault' | 'locked' | 'unlocked', summary?: VaultSummary }`
- `entries` — current page/filtered list, kept minimal (list view uses lightweight `EntrySummary`, full secrets fetched on demand).
- `searchQuery` — debounced text driving `list_entries`.
- `settings` — auto-lock timeout, lock-on-blur toggle, clipboard-clear delay, theme prefs.
- `toast` — transient notifications.

The frontend requests full entry details (including secrets) only when an entry is opened, minimizing the amount of decrypted secret material held in the webview at any time.

## Data Models

### Decrypted Vault Model (in memory / encrypted payload)

```jsonc
{
  "schema_version": 1,
  "entries": [
    {
      "id": "uuid-v4",
      "service_ref": { "kind": "catalog", "id": "facebook" },
      // or { "kind": "custom", "id": "custom-uuid" }
      "title": "Personal Facebook",          // optional user label
      "fields": [
        { "id": "f1", "label": "Email",    "type": "email",    "value": "...", "secret": false },
        { "id": "f2", "label": "Password", "type": "password", "value": "...", "secret": true  },
        { "id": "f3", "label": "Phone",    "type": "phone",    "value": "...", "secret": false }
      ],
      "created_at": "ISO-8601",
      "updated_at": "ISO-8601"
    }
  ],
  "custom_services": [
    { "id": "custom-uuid", "name": "My Home Server", "icon": { "kind": "builtin|data", "ref": "..." } }
  ]
}
```

Field `type` values: `email | username | password | phone | url | text | note | totp_secret | recovery_code`. The `secret` flag controls masking, clipboard auto-clear, and exclusion from the search index.

### Bundled Catalog Entry

```jsonc
{
  "id": "facebook",
  "name": "Facebook",
  "icon": "facebook.svg",
  "aliases": ["fb", "meta"],
  "recommended_fields": [
    { "label": "Email",    "type": "email",    "secret": false },
    { "label": "Phone",    "type": "phone",    "secret": false },
    { "label": "Password", "type": "password", "secret": true  }
  ]
}
```

The catalog ships as `catalog/services-catalog.json` plus an `icons/` directory, generated by a **dev-time Node.js script** (`tools/build-catalog.mjs`) combined with manual curation. The script is never shipped to users; only its JSON/icon output is bundled.

### Settings (stored encrypted within the vault)

```jsonc
{
  "auto_lock_seconds": 300,        // 0 = disabled
  "lock_on_blur": false,
  "clipboard_clear_seconds": 20,
  "password_gen_defaults": { "length": 20, "upper": true, "lower": true, "digits": true, "symbols": true }
}
```

## Vault File Format (`.khv`)

A single binary file with a plaintext (non-secret) header followed by the encrypted payload. The header carries everything needed to derive the key; it contains no secrets.

```
┌──────────────────────────────────────────────────────────────┐
│ MAGIC        "KHVAULT\0"            (8 bytes)                   │
│ FORMAT_VER   u16                    (file format version)      │
│ CIPHER_ID    u8                     (1 = XChaCha20-Poly1305)   │
│ KDF_ID       u8                     (1 = Argon2id)             │
│ KDF_PARAMS   { m_cost, t_cost, p_cost }  (Argon2 parameters)   │
│ MASTER_SALT  16 bytes               (salt for master pw KDF)   │
│ ── Key-wrapping section (envelope encryption) ──               │
│ PW_WRAP      { nonce, ciphertext }  (VEK wrapped by master key)│
│ HAS_RECOVERY u8                     (0/1)                      │
│ REC_SALT     16 bytes               (present if recovery)      │
│ REC_WRAP     { nonce, ciphertext }  (VEK wrapped by rec. key)  │
│ ── Payload ──                                                  │
│ PAYLOAD_NONCE 24 bytes                                         │
│ PAYLOAD       AEAD ciphertext of compressed JSON vault model   │
│ (AEAD auth tags integrity-protect each encrypted section)      │
└──────────────────────────────────────────────────────────────┘
```

### Envelope Encryption (why two "wraps")

A single random **Vault Encryption Key (VEK)** encrypts the payload. The VEK itself is then encrypted ("wrapped") separately by:
1. a key derived from the **master password** (Argon2id over `MASTER_SALT`), and
2. (optionally) a key derived from the **recovery key** (Argon2id over `REC_SALT`).

This is what lets either secret unlock the same vault independently, and lets us **change the master password without re-encrypting the whole payload** (only the password wrap is rewritten). Tampering with any section is caught by the AEAD authentication tags.

### Crypto Flows

**Create vault**
1. Generate random VEK (32 bytes) and `MASTER_SALT`.
2. `master_key = Argon2id(master_password, MASTER_SALT, params)`.
3. `PW_WRAP = AEAD_encrypt(master_key, VEK)`.
4. If recovery requested: generate recovery key + `REC_SALT`; `rec_key = Argon2id(recovery_key, REC_SALT)`; `REC_WRAP = AEAD_encrypt(rec_key, VEK)`.
5. `PAYLOAD = AEAD_encrypt(VEK, compress(serialize(vault_model)))`.
6. Write header + payload atomically. Zeroize `master_key`, `rec_key`, passwords.

**Unlock (password)**
1. Read header; `master_key = Argon2id(master_password, MASTER_SALT, params)`.
2. `VEK = AEAD_decrypt(master_key, PW_WRAP)` → failure means wrong password (auth tag fails).
3. `vault_model = deserialize(decompress(AEAD_decrypt(VEK, PAYLOAD)))`.
4. Hold VEK + model in session memory; zeroize `master_key` and password.

**Unlock (recovery key)**: same as above but derive from `REC_SALT`/`REC_WRAP`, then prompt to set a new master password (recomputes `PW_WRAP`).

**Change master password**: unlock to obtain VEK → derive new master key from new password + fresh salt → rewrite only `MASTER_SALT` + `PW_WRAP`. Payload untouched.

**Save (any write)**: re-encrypt payload with VEK and a fresh `PAYLOAD_NONCE`, write to a temp file, fsync, then atomically rename over the vault (crash-safe), keeping a `.bak` of the prior version.

## Auto-Lock Design

- The Rust session manager owns an inactivity deadline. The frontend reports "activity" (debounced) via a lightweight command/event; the backend resets the deadline.
- A backend timer task checks the deadline; on expiry it locks: drops decrypted model + VEK, zeroizes, and emits a `vault-locked` event the frontend listens for to route back to the Unlock screen.
- If `lock_on_blur` is enabled, the Tauri window blur/minimize event triggers the same lock path (optionally after a short grace threshold).
- Keeping the authoritative timer in the backend means the lock cannot be bypassed by freezing the webview.

## Clipboard Auto-Clear

- `copy_secret_to_clipboard` copies via Tauri's clipboard API and schedules a backend task to overwrite the clipboard after `clipboard_clear_seconds`, but only if the clipboard still contains the copied value (avoid clobbering unrelated content the user copied since).

## Search & Performance at Scale

- The backend keeps an in-memory search index built from **non-secret** fields only (service name, labels, non-secret values). Secret values are never indexed (Req 9.4).
- `list_entries` returns lightweight `EntrySummary` objects (id, service, title, snippet) — not full secrets — and supports pagination/offset.
- The Svelte list uses **virtualized rendering** so only visible rows mount, keeping thousands of entries smooth.
- Search queries are debounced (~120ms) on the frontend; filtering runs in Rust against the prebuilt index.

## UI / UX Design System

### Visual Language (calm, minimalist, Apple-inspired)

- **Backgrounds**: soft off-white (`#F7F9FC`) rather than harsh pure white; cards in `#FFFFFF` with subtle elevation.
- **Primary accent (blue)**: a calm, desaturated blue (`#3B82C4` / hover `#2F6DA8`); used sparingly for primary actions and selection.
- **Text**: near-black slate (`#1F2933`) for primary, muted slate (`#5B6B7B`) for secondary — avoids harsh pure-black-on-white glare.
- **Borders/dividers**: very light cool gray (`#E3E8EF`).
- **Status colors**: muted green (success), muted amber (warning), muted red (danger) — all low-saturation to stay easy on the eyes.
- **Radius**: 12–16px on cards/inputs for a soft, modern feel.
- **Shadows**: soft, low-opacity, short-spread for gentle depth (no harsh drop shadows).
- **Typography**: system font stack (`-apple-system, "Segoe UI", Inter, system-ui, sans-serif`); clear hierarchy, generous line-height.
- **Spacing**: generous whitespace; an 8px spacing scale.
- **Motion**: Svelte `fade`/`fly`/`scale` with short durations (120–200ms) and ease-out; spring on small interactive elements. Respect `prefers-reduced-motion`.

Design tokens live in `lib/design/tokens.css` (CSS custom properties) so the palette is centrally tunable. Although the brief is light-theme-first, tokens are structured so a future dark theme is feasible without rework.

### Key Screens

1. **Setup** (first run): create master password (with strength meter), optional recovery-key generation with one-time reveal + confirm-saved gate + security warning.
2. **Unlock**: master password field, "use recovery key instead" link, clear error on failure.
3. **Recovery**: enter recovery key → set new master password.
4. **Vault (main)**: left/top search bar, virtualized entry list grouped/identifiable by service icon, prominent "Add" button, manual "Lock" control.
5. **Entry editor**: service picker (searchable catalog + "Create custom service"), recommended fields prefilled, add/remove fields, password fields masked with reveal/copy + inline generator.
6. **Settings**: auto-lock timeout, lock-on-blur, clipboard-clear delay, password-generator defaults, export/import & backup guidance.

### Accessibility

- Maintain WCAG AA contrast for text and essential controls (verified against the chosen palette).
- Full keyboard navigation; visible focus rings; ARIA labels on icon-only buttons.
- Respect `prefers-reduced-motion` (disable non-essential animation).
- Note: full WCAG conformance requires manual testing with assistive technology; the design targets AA but final validation is a manual step.

## Error Handling

A single `KeyhavenError` enum in Rust, mapped to user-friendly messages in the frontend:

| Variant | Cause | UX behavior |
|---|---|---|
| `WrongCredentials` | AEAD unwrap of VEK fails | "Incorrect master password / recovery key." No crash. |
| `VaultCorrupted` | Magic/version mismatch or payload auth-tag failure | "This vault file is damaged or not a Keyhaven vault." Offers `.bak` restore if present. |
| `IncompatibleVersion` | `FORMAT_VER` newer than supported | Prompt to update Keyhaven; never attempt to overwrite. |
| `Locked` | Entry command called while locked | Route to Unlock screen. |
| `Io` | File read/write/permission issues | Clear message + path; suggest a different location. |
| `InvalidInput` | Bad command arguments | Inline validation message. |

- Writes are atomic (temp + fsync + rename) so a crash mid-save cannot corrupt the existing vault; the previous version is retained as `.bak`.
- Import validates magic/version before touching any existing vault (Req 11.6); a failed import never alters the current vault.

## Security Considerations

- **Crypto crates (audited, no custom crypto)**: `argon2`, `chacha20poly1305` (and/or `aes-gcm`), `rand` (CSPRNG via `getrandom`), `zeroize`. (Req 15.6)
- **Secret lifetime**: master password, recovery key, derived keys, and VEK are wrapped in `Zeroizing` types and cleared as soon as they are no longer needed and on lock/exit. (Req 15.5)
- **No plaintext at rest**: only the non-secret header (salts, KDF params, nonces) is unencrypted; all credential data is inside the AEAD payload. (Req 15.3)
- **Tamper detection**: every encrypted section carries an authentication tag; modification is rejected. (Req 15.4)
- **No network**: Tauri's allowlist/capabilities are configured to forbid HTTP and any networking; the frontend has no fetch/sync code. (Req 13.1, 13.2)
- **IPC minimization**: the webview holds full secrets only for the single entry being viewed/edited, fetched on demand.
- **Argon2 parameters**: tuned to a target derivation time (~250–500ms) on typical hardware; parameters are stored in the header so existing vaults remain openable if defaults change later.

## Correctness Properties

These are invariants the implementation must uphold; they drive the test suite and any property-based tests.

### Property 1: Round-trip fidelity
For any valid vault model `m`, `decrypt(encrypt(m))` deserializes to a model equal to `m` (entries, fields, custom services, settings all preserved).

**Validates: Requirements 5.4, 7.6, 11.4**

### Property 2: Confidentiality at rest
The on-disk vault file contains no credential value, master password, recovery key, or derived key in plaintext or any reversible-without-secret form. Only the non-secret header (salts, KDF params, nonces, cipher/KDF ids) is unencrypted.

**Validates: Requirements 1.8, 15.1, 15.3**

### Property 3: Authentication / tamper-evidence
Any single-byte modification to any encrypted section (PW_WRAP, REC_WRAP, or PAYLOAD) causes decryption to fail rather than return wrong or partial data.

**Validates: Requirements 3.6, 15.4**

### Property 4: Wrong-secret rejection
Unlock with an incorrect master password or recovery key fails via AEAD auth-tag failure and never yields a usable VEK.

**Validates: Requirements 2.8, 3.3**

### Property 5: Either-key independence
When a recovery key exists, the master password and the recovery key each independently unlock the same VEK; neither requires the other.

**Validates: Requirements 2.3, 2.7**

### Property 6: Change-password isolation
Changing the master password rewrites only `MASTER_SALT` + `PW_WRAP`; the payload ciphertext and the recovery wrap remain valid (recovery key still works, entries unchanged).

**Validates: Requirements 2.7, 3.2**

### Property 7: Search excludes secrets
No field marked `secret: true` ever appears in the search index or in `list_entries` summaries.

**Validates: Requirements 9.4**

### Property 8: Atomic writes
A failure at any point during a save leaves either the previous complete vault or a recoverable `.bak`; the file is never left partially written and unreadable.

**Validates: Requirements 8.5, 11.6**

### Property 9: Import safety
Importing an invalid or foreign file fails validation before any modification to an existing vault; the current vault is never altered by a failed import.

**Validates: Requirements 11.6**

### Property 10: Secret zeroization
After `lock_vault` or app exit, derived keys, the VEK, and entered secrets are zeroized (best-effort) and no longer resident in the session.

**Validates: Requirements 3.5, 15.5**

### Property 11: Generator compliance
A generated password satisfies the requested length and only draws from the selected character sets, using a CSPRNG.

**Validates: Requirements 10.2, 10.3**

## Testing Strategy

**Rust (backend) — the security-critical core gets the most coverage:**
- Unit tests: Argon2id derivation determinism, AEAD encrypt/decrypt round-trips, wrong-key failure, tamper detection (flip a byte → decryption fails).
- Vault round-trip: create → write → read → unlock → contents match, for both password and recovery-key paths.
- Change-password: payload unchanged, old password rejected, new accepted, recovery still works.
- Atomic-write/crash-safety: simulate failure mid-write, verify `.bak`/recovery.
- Import validation: malformed/foreign files rejected without touching existing vault.
- Password generator: respects length/charset options; uses CSPRNG.
- Property tests where useful (e.g., arbitrary vault models survive serialize→encrypt→decrypt→deserialize).

**Frontend (Svelte):**
- Component tests for EntryEditor field add/remove, masking/reveal, service picker, generator UI (Vitest + Testing Library).
- Store logic for debounced search and session state transitions.

**Integration (Tauri):**
- Smoke tests of command handlers (create/unlock/CRUD/lock) against a temp vault file.

**Manual:**
- Cross-platform launch verification (Windows/macOS/Linux), accessibility/contrast and reduced-motion checks.

> Per project guidance, automated tests will be added when implementing the corresponding tasks rather than retrofitted; the test items above are folded into the task list.
