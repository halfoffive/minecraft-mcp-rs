#!/usr/bin/env node
// sync-versions.mjs — stamp the Cargo.toml version into every npm package.
//
// The version source of truth is Cargo.toml (the Rust release process bumps
// it in T22; npm must never drift). This script:
//   1. reads `version = "x.y.z"` from the root Cargo.toml,
//   2. writes that version into every npm/*/package.json,
//   3. for the main package, also stamps every optionalDependencies value
//      (they must always match the main version — platform packages are
//      released in lockstep).
//
// Usage: node npm/scripts/sync-versions.mjs
// Exit code 1 with a message if Cargo.toml is unreadable or the version
// cannot be extracted.

import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CARGO_TOML = join(ROOT, "Cargo.toml");

let cargoText;
try {
  cargoText = readFileSync(CARGO_TOML, "utf8");
} catch (err) {
  console.error(`sync-versions: cannot read ${CARGO_TOML}: ${err.message}`);
  process.exit(1);
}

const match = cargoText.match(/^version\s*=\s*"([^"]+)"/m);
if (!match) {
  console.error('sync-versions: could not find a `version = "..."` line in Cargo.toml');
  process.exit(1);
}
const version = match[1];

const npmRoot = join(ROOT, "npm");
const packageDirs = readdirSync(npmRoot)
  .filter((name) => name.startsWith("minecraft-mcp-rs"))
  .filter((name) => statSync(join(npmRoot, name)).isDirectory());

if (packageDirs.length === 0) {
  console.error("sync-versions: no npm package directories found under npm/");
  process.exit(1);
}

for (const dir of packageDirs) {
  const pkgPath = join(npmRoot, dir, "package.json");
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
  pkg.version = version;
  if (pkg.optionalDependencies) {
    for (const key of Object.keys(pkg.optionalDependencies)) {
      pkg.optionalDependencies[key] = version;
    }
  }
  writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);
  console.log(`sync-versions: ${dir} -> ${version}`);
}
