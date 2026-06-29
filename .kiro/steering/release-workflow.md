---
inclusion: always
---

# Keyhaven release workflow (agreed with the owner)

Always follow this order when shipping an update. Confirmed by the owner; use it
by default for every release.

## The order: Test → Commit → Release → CI builds

1. **Make the changes** (features/fixes).
2. **Test locally first.** Use `npm run tauri dev` (fast) or `npm run tauri build`
   to smoke-test the installer. The version label does NOT need to be bumped to
   test — it's just a label; the local build compiles the current working tree.
3. **Commit & push** the code (only on the owner's go signal — see below).
4. **Release:** `npm run release <X.Y.Z>` — this bumps the version across all
   manifests, commits, tags `vX.Y.Z`, and pushes. That tag triggers CI.
5. **CI builds the official, signed installers** (Windows/macOS/Linux) + updater
   `latest.json`/`.sig` and publishes the GitHub Release. Users on the previous
   signed version auto-update on next launch.

## Key rules

- **Do NOT distribute local builds.** The official artifacts users download are
  built by GitHub Actions (CI), never uploaded manually. A local
  `npm run tauri build` is for the owner's testing only.
- **Do NOT bump the version before testing.** The version is a release label;
  `npm run release` sets it at release time. Bumping early just creates confusion.
- **Version choice (SemVer):** bug fix → patch (1.1.0→1.1.1); new feature →
  minor (1.1.0→1.2.0); breaking change → major (1.x→2.0.0). When unsure, ask the
  owner or default to minor. The owner often prefers the AI to pick the number.
- **Local build "no private key" error is harmless** — signing happens in CI
  (which holds the `TAURI_SIGNING_PRIVATE_KEY` secret), not on the laptop.
- **Never regenerate the updater signing key** (`keyhaven-updater.key`). The
  public half is baked into installed apps; a new key would make existing
  installs reject future updates. Keep the keypair stable for the app's life.
- **Wait for the owner's explicit "go" before committing/pushing/releasing.**
  The owner reviews/tests first, then signals when to ship.

## Project facts

- Repo: `Mixscoo/keyhaven` (public — required so releases are downloadable and
  the auto-updater can fetch `latest.json`).
- Release helper: `tools/release.mjs` (run via `npm run release <version>`); it
  guards against downgrades and existing tags before changing anything.
- Current released version baseline: see `src-tauri/tauri.conf.json` (`version`).
