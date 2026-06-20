#!/usr/bin/env node
/*
 * Keyhaven release helper.
 *
 * Bumps the version across every manifest, commits, tags, and pushes — which
 * triggers the GitHub Actions release workflow (build installers + publish the
 * GitHub Release).
 *
 *   npm run release 1.1.0
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

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("Usage: npm run release <version>   e.g.  npm run release 1.1.0");
  process.exit(1);
}
const tag = `v${version}`;

const read = (p) => readFileSync(join(ROOT, p), "utf8");
const write = (p, c) => writeFileSync(join(ROOT, p), c);

// --- 1. JSON manifests (preserve 2-space formatting) ---
for (const p of ["package.json", "package-lock.json", "src-tauri/tauri.conf.json"]) {
  const json = JSON.parse(read(p));
  json.version = version;
  if (json.packages && json.packages[""]) json.packages[""].version = version;
  write(p, JSON.stringify(json, null, 2) + "\n");
}

// --- 2. Cargo.toml ([package] version, scoped to the keyhaven package) ---
write(
  "src-tauri/Cargo.toml",
  read("src-tauri/Cargo.toml").replace(
    /(name = "keyhaven"\s*\nversion = ")[^"]+(")/,
    `$1${version}$2`,
  ),
);

// --- 3. Cargo.lock (the keyhaven package entry only) ---
write(
  "src-tauri/Cargo.lock",
  read("src-tauri/Cargo.lock").replace(
    /(\[\[package\]\]\nname = "keyhaven"\nversion = ")[^"]+(")/,
    `$1${version}$2`,
  ),
);

console.log(`Version set to ${version} across all manifests.`);

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
