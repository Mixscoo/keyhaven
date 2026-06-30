#!/usr/bin/env node
/*
 * Keyhaven release-notes builder (CI-only helper).
 *
 * Reads CHANGELOG.md, extracts the section for a given version, and composes
 * the GitHub Release body: the user-facing changelog first, then the download
 * instructions. The result is written to the GitHub Actions step output
 * `body` (when running in CI) and also printed to stdout (for local preview).
 *
 *   node tools/release-notes.mjs v1.2.1
 *   node tools/release-notes.mjs 1.2.1
 *
 * Writing the step output from Node keeps this cross-platform: it works the
 * same on the Linux, macOS, and Windows runners without shell-specific heredoc
 * syntax. If the version has no changelog entry, a sensible generic body is
 * used instead of failing the release.
 */
import { readFileSync, appendFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// Accept either "v1.2.1" or "1.2.1".
const raw = (process.argv[2] || "").trim();
const version = raw.replace(/^v/, "");
const tag = `v${version}`;

/**
 * Pull the body of the `## [x.y.z] ...` section out of CHANGELOG.md, stopping
 * at the next `## ` heading. Returns "" when the version isn't found.
 */
function extractSection(markdown, ver) {
  const lines = markdown.split(/\r?\n/);
  // Match "## [1.2.1] - ..." or "## 1.2.1 - ..." (brackets optional).
  const esc = ver.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const headingRe = new RegExp(`^##\\s+\\[?${esc}\\]?(\\s|$)`);
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (headingRe.test(lines[i])) {
      start = i + 1;
      break;
    }
  }
  if (start === -1) return "";
  let end = lines.length;
  for (let i = start; i < lines.length; i++) {
    if (/^##\s+/.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.slice(start, end).join("\n").trim();
}

const DOWNLOADS = `### Downloads

Pick the installer for your platform from the assets below.

- **Windows:** \`.exe\` (NSIS) or \`.msi\`
- **macOS:** \`.dmg\` (Intel and Apple Silicon)
- **Linux:** \`.AppImage\` or \`.deb\`

If you're already running a previous signed version, the app updates itself
automatically on next launch.

> Note: builds are not code-signed yet, so your OS may show an
> "unknown publisher" / Gatekeeper prompt on first launch.`;

let changelog = "";
try {
  const md = readFileSync(join(ROOT, "CHANGELOG.md"), "utf8");
  changelog = extractSection(md, version);
} catch {
  /* no changelog file — fall back to generic notes */
}

const header = `## Keyhaven ${tag}

Offline-first, encrypted password manager.`;

const whatsNew = changelog
  ? `### What's new\n\n${changelog}`
  : `This release includes general improvements and fixes.`;

const body = `${header}\n\n${whatsNew}\n\n---\n\n${DOWNLOADS}\n`;

// Always print for local preview / CI logs.
console.log(body);

// In CI, expose it as the step output `body` using a unique heredoc delimiter
// so multi-line content is captured correctly.
const out = process.env.GITHUB_OUTPUT;
if (out) {
  const delim = `EOF_${Math.random().toString(36).slice(2)}`;
  appendFileSync(out, `body<<${delim}\n${body}\n${delim}\n`);
}
