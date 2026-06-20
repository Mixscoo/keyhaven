<div align="center">
  <img src="public/keyhaven.svg" width="84" height="84" alt="Keyhaven" />
  <h1>Keyhaven</h1>
  <p><strong>An offline-first, cross-platform, encrypted password manager.</strong></p>
</div>

Keyhaven stores your credentials in a single, portable, strongly-encrypted vault
file on your own device. It runs **fully offline** — there is no account, no
sync server, and no network access of any kind.

## Features

- **Strong, modern cryptography** — Argon2id key derivation and
  XChaCha20-Poly1305 authenticated encryption. Your master password is never
  stored anywhere; it only ever derives the key that unwraps your vault.
- **Local & portable** — the whole vault is one `.khv` file you can back up,
  move, or import on another machine.
- **Recovery key** — an optional one-time key to recover your vault if you
  forget your master password (save it to a file in one click).
- **Service catalog with real logos** — pick from popular services (with their
  brand icons) and get sensible prefilled fields, or define your own.
- **Password generator** with configurable length and character sets.
- **Auto-lock** on inactivity or window blur, and **clipboard auto-clear** that
  also excludes copied secrets from Windows Clipboard History and Cloud
  Clipboard.
- **Import/export** for moving your vault between devices.

## Install

Download the installer for your platform from the
[latest release](https://github.com/Mixscoo/keyhaven/releases/latest):

- **Windows** — `.exe` (NSIS) or `.msi`
- **macOS** — `.dmg` (Intel and Apple Silicon builds)
- **Linux** — `.AppImage` or `.deb`

> Builds are not code-signed yet, so your OS may warn about an "unknown
> publisher" (Windows SmartScreen) or block first launch (macOS Gatekeeper).
> You can allow it through the OS prompts.

## Security model

- The vault file contains only the **encrypted** vault key and your encrypted
  data — never your master password, the recovery key, or any plaintext secret.
- A leaked vault file is useless without your master password or recovery key.
- All cryptography runs in a trusted Rust core; the UI never performs crypto or
  networking.

Keyhaven has **not** had an independent security audit. Use it at your own risk
and keep backups of your vault.

## Build from source

Requires [Node.js](https://nodejs.org) 20+ and the
[Rust toolchain](https://rustup.rs), plus the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev     # run in development
npm run tauri build   # build installers for your platform
```

## Tech stack

[Tauri 2](https://tauri.app) (Rust backend) + [Svelte 5](https://svelte.dev)
(TypeScript frontend).

## License

See the repository for license details.
