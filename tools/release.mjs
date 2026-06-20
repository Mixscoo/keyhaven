#!/usr/bin/env node
/*
 * Keyhaven release helper.
 *
 * Bumps the version across every manifest, commits, tags, and pushes — which
 * triggers the GitHub Actions release workflow (build installers + publish the
 * GitHub Release).
 *
 *   npm run release 1.2.0
 *
 * Safety guards (checked BEFORE anything is changed, so a rejected release
 * leaves your repo untouched):
 *   - the version must be valid semver (x.y.z),
 *   - it must be strictly HIGHER than the current version (no accidental
 *     downgrade / re-release),
 *   - the matching git tag must not already exist (locally or on the remote).
 *
 * Run it on a clean working tree (everything you want shipped already
 * committed). It stages all changes, so make sure the tree only contains what
 * you intend to release.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const read = (p) => readFileSync(join(ROOT, p), "utf8");
const write = (p, c) => writeFileSync(join(ROOT, p), c);

function fail(message) {
  console.error(`\n✗ Release aborted: ${message}\n  (Nothing was changed.)`);
  process.exit(1);
}

/** Run a command and capture stdout; return "" on any failure. */
function capture(cmd) {
  try {
    return execSync(cmd, { cwd: ROOT, stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim();
  } catch {
    return "";
  }
}

/** Compare two x.y.z strings: negative if a<b, 0 if equal, positive if a>b. */
function cmpSemver(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] - pb[i];
  }
  return 0;
}

// ---------------------------------------------------------------------------
// Validate (no side effects until every check passes)
// ---------------------------------------------------------------------------
const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  fail(`"${version ?? ""}" is not a valid version. Use x.y.z, e.g. 1.2.0`);
}
const tag = `v${version}`;

// Current version is read from tauri.conf.json (the source of truth the app and
// installers use).
const current = JSON.parse(read("src-tauri/tauri.conf.json")).version;
if (cmpSemver(version, current) <= 0) {
  fail(
    `${version} is not higher than the current version ${current}. ` +
      `Pick a larger number (e.g. a patch ${bump(current, "patch")}, ` +
      `minor ${bump(current, "minor")}, or major ${bump(current, "major")}).`,
  );
}

if (capture(`git tag -l ${tag}`) || capture(`git ls-remote --tags origin refs/tags/${tag}`)) {
  fail(`tag ${tag} already exists. Pick a new version number.`);
}

// ---------------------------------------------------------------------------
// Apply the bump across all manifests
// ---------------------------------------------------------------------------
for (const p of ["package.json", "package-lock.json", "src-tauri/tauri.conf.json"]) {
  const json = JSON.parse(read(p));
  json.version = version;
  if (json.packages && json.packages[""]) json.packages[""].version = version;
  write(p, JSON.stringify(json, null, 2) + "\n");
}

write(
  "src-tauri/Cargo.toml",
  read("src-tauri/Cargo.toml").replace(
    /(name = "keyhaven"\s*\nversion = ")[^"]+(")/,
    `$1${version}$2`,
  ),
);

write(
  "src-tauri/Cargo.lock",
  read("src-tauri/Cargo.lock").replace(
    /(\[\[package\]\]\nname = "keyhaven"\nversion = ")[^"]+(")/,
    `$1${version}$2`,
  ),
);

console.log(`Version ${current} -> ${version} across all manifests.`);

// ---------------------------------------------------------------------------
// Commit, tag, push
// ---------------------------------------------------------------------------
const run = (cmd) => {
  console.log(`$ ${cmd}`);
  execSync(cmd, { cwd: ROOT, stdio: "inherit" });
};

const branch = execSync("git rev-parse --abbrev-ref HEAD", { cwd: ROOT })
  .toString()
  .trim();

run("git add -A");
run(`git commit -m "Release ${tag}"`);
run(`git tag -a ${tag} -m "Keyhaven ${tag}"`);
run(`git push origin ${branch}`);
run(`git push origin ${tag}`);

console.log(`\n✓ Released ${tag}. GitHub Actions is now building installers.`);
console.log(`  Release: https://github.com/Mixscoo/keyhaven/releases/tag/${tag}`);

/** Suggest the next patch/minor/major from a base x.y.z (for the error hint). */
function bump(base, kind) {
  const [maj, min, pat] = base.split(".").map(Number);
  if (kind === "major") return `${maj + 1}.0.0`;
  if (kind === "minor") return `${maj}.${min + 1}.0`;
  return `${maj}.${min}.${pat + 1}`;
}
