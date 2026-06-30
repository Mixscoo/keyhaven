# Changelog

All notable changes to Keyhaven are documented here. The newest version is
listed first. Dates use the `YYYY-MM-DD` format.

This project follows [Semantic Versioning](https://semver.org): bug fixes bump
the patch number, new features bump the minor number, and breaking changes bump
the major number.

## [1.2.1] - 2026-06-30

### Fixed
- **Copy button now works reliably.** Copying a saved password (or any field)
  now places it on the clipboard so you can paste it anywhere. Copied secrets
  are still kept out of Windows Clipboard History (Win+V) and Cloud Clipboard,
  and the clipboard still auto-clears after the configured delay.

### Added
- **Threads (Instagram Threads)** is now in the service catalog, with its own
  logo and recommended fields.

### Changed
- **Crisper Coins.ph and PLDT logos.** Both now use high-resolution artwork
  instead of the small, blurry website favicons.

## [1.2.0] - 2026-06-29

### Added
- Many new services in the catalog, including Social Club (Rockstar), EA/Origin,
  Sellix, Coins.ph, PLDT, RCBC, OKX, MetaMask, ChatGPT, Payoneer, Kiro,
  DataBlitz, and OnlineJobs.ph.

### Changed
- **Reorganized the main list.** Entries are grouped by service. Services with a
  single account open straight to the entry; services with several accounts
  collapse into one tidy group you can expand.
- **View-first editing.** Opening an entry now shows a read-only View. You tap
  **Edit** to make changes, so you can't accidentally alter a field.
- **Friendlier Settings.** The password-length control gained a slider with
  steppers and a number box, and the character-set options became toggle chips.
- App-wide custom scrollbars, a padlock icon for Lock, and assorted polish.

### Fixed
- Removed the duplicate show/hide icon that appeared inside secret fields.
- Stopped saving empty fields when you save an entry.

## [1.1.0] - 2026-06-28

### Added
- **Automatic updates.** Installed apps check for new signed releases on launch
  and can update themselves, so you stay current without reinstalling.

## [1.0.0] - 2026-06-27

### Added
- First release of Keyhaven: an offline-first, encrypted password manager.
  Your vault is protected with Argon2id + XChaCha20-Poly1305 and never leaves
  your device.
