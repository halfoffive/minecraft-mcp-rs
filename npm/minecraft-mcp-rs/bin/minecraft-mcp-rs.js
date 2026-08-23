#!/usr/bin/env node
"use strict";

// minecraft-mcp-rs — platform-package launcher shim.
//
// The main npm package is platform-independent; the actual native binary
// lives in one of five optional platform packages (see `optionalDependencies`
// in package.json). This shim resolves the platform package for the current
// process.platform/process.arch, locates the binary inside it, and execs it,
// forwarding all CLI arguments.
//
// If the platform package is missing (e.g. the user installed with
// `--omit=optional` or on an unsupported platform), we print a helpful error
// and exit 1 instead of crashing with an unhelpful stack trace.

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");

// Map of `${process.platform}-${process.arch}` to [packageName, binaryName].
// The binary names match what the CI npm-publish job stages into each
// platform package (see .github/workflows/release.yml).
const PLATFORMS = {
  "win32-x64": ["@minecraft-mcp-rs/minecraft-mcp-rs-windows-x64", "minecraft-mcp-rs.exe"],
  "win32-arm64": ["@minecraft-mcp-rs/minecraft-mcp-rs-windows-arm64", "minecraft-mcp-rs.exe"],
  "darwin-arm64": ["@minecraft-mcp-rs/minecraft-mcp-rs-darwin-arm64", "minecraft-mcp-rs"],
  "linux-x64": ["@minecraft-mcp-rs/minecraft-mcp-rs-linux-x64", "minecraft-mcp-rs"],
  "linux-arm64": ["@minecraft-mcp-rs/minecraft-mcp-rs-linux-arm64", "minecraft-mcp-rs"],
};

const platformKey = `${process.platform}-${process.arch}`;
const entry = PLATFORMS[platformKey];

if (!entry) {
  console.error(
    `minecraft-mcp-rs does not support ${process.platform} (${process.arch}).\n` +
      `Supported platforms: ${Object.keys(PLATFORMS).join(", ")}.`
  );
  process.exit(1);
}

const [pkgName, binaryName] = entry;

let binPath;
try {
  // resolve() throws if the platform package is not installed.
  binPath = require.resolve(`${pkgName}/${binaryName}`);
} catch {
  console.error(
    `The ${pkgName} platform package is missing (expected binary: ${binaryName}).\n` +
      `This usually means npm did not install optional dependencies:\n` +
      `  - did you use \`npm install --omit=optional\`? That breaks the platform packages.\n` +
      `  - reinstall with: npm install ${pkgName}\n` +
      `  - or reinstall the main package with --force to re-fetch optional deps.`
  );
  process.exit(1);
}

// Spawn options shared by both attempts below. `windowsHide` keeps a console
// window from flashing when the shim is launched from an MCP client host on
// Windows; stdio stays inherited either way (in --stdio mode stdout IS the
// MCP JSON-RPC transport and must reach the parent untouched).
const spawnOptions = { stdio: "inherit", windowsHide: true };

let result = spawnSync(binPath, process.argv.slice(2), spawnOptions);

// npx/bunx cache extraction can lose the executable bit on POSIX systems
// (tarball mode bits are not always preserved through the cache). A spawn
// that fails with EACCES/ENOEXEC is retried once after restoring +x, so
// `npx -y minecraft-mcp-rs` self-heals instead of dead-ending.
if (
  result.error &&
  process.platform !== "win32" &&
  (result.error.code === "EACCES" || result.error.code === "ENOEXEC")
) {
  try {
    fs.chmodSync(binPath, 0o755);
    result = spawnSync(binPath, process.argv.slice(2), spawnOptions);
  } catch {
    // chmod failed (read-only store, etc.) — fall through to the error exit.
  }
}

if (result.error) {
  console.error(`Failed to launch ${binPath}: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
