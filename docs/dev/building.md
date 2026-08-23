# Building from Source

This guide covers building, running, and testing `minecraft-mcp-rs` from
source. If you only want to **use** the server, skip all of this — the
[npm install](../npm) page gets you running with `npx`/`bunx` and no Rust
toolchain.

## Prerequisites

- [Rust nightly](https://rustup.rs/) — the version is pinned by date in
  `rust-toolchain.toml`; any recent `rustup` picks it up automatically when
  you enter the repository.
- Node.js ≥ 18 — only needed for the docs site (`npm run docs:dev`) and the
  npm launcher shim; not needed to compile the binary itself.
- A **Minecraft Java Edition 1.21.11** server (local or remote) — **the only
  supported version**. azalea 0.15.1 targets exactly this protocol version;
  a different server will reject the bot at login.

## Why nightly is pinned

The crate builds on Rust edition 2024, and azalea 0.15.1's build script
enforces a nightly compiler. The toolchain file pins a nightly **date**
(e.g. `nightly-2026-05-28`) rather than the moving `nightly` channel: a bare
`nightly` let a fresh upstream compiler break `azalea-core` in CI with E0284
in `FixedBitSet`. When upgrading dependencies that touch azalea, bump the
pinned date as part of the same change and verify the full gate suite below
still passes.

## Build

```bash
cargo build          # dev profile
cargo build --release
```

The dev profile uses `opt-level = 1` for workspace code but keeps
dependencies at `opt-level = 3`, so iteration stays fast while azalea's
hot loops (chunk decoding, pathfinding) remain usable at runtime.

## Run

```bash
cargo run                 # prints usage, exits 0
cargo run -- --gui        # desktop UI (egui window)
cargo run -- --headless --stdio   # headless MCP server on stdio
cargo run -- --headless           # headless MCP server on HTTP (default)
```

See [Getting Started](../guide/getting-started) for connecting an MCP
client, and [Configuration](../config) for every `MINECRAFT_MCP_*`
environment variable.

## Test

```bash
cargo test                         # everything below
cargo test --lib                   # unit tests only
cargo test --test integration      # mock-based end-to-end tests
cargo test --test proptest         # property-based tests
```

Unit tests live at the bottom of each source file in
`#[cfg(test)] mod tests`. Integration tests use mocks (no real Minecraft
server), so the whole suite runs offline in well under a minute.

## Lint & docs gates

CI enforces all of these; run them locally before opening a PR:

```bash
cargo fmt                       # formatting check / apply
cargo clippy --all-targets      # must be zero-warning
cargo doc --no-deps             # must be zero-warning ([lints.rustdoc] deny baseline)
```

The rustdoc gate denies broken intra-doc links, private-link misuse, and
redundant explicit links — link private items as plain code spans
(`` `item` ``), not `[]` links.

## Cross-compiling / release artifacts

Release binaries are built by CI (`.github/workflows/release.yml`) for
Linux x64/arm64, macOS arm64, and Windows x64/arm64, then staged into the
npm platform packages. To reproduce one locally:

```bash
cargo build --release
# target/release/minecraft-mcp-rs(.exe)
```

Native ARM runners build the ARM targets — no cross toolchain involved.

## Common build issues

| Symptom | Cause / fix |
|---------|-------------|
| `E0284` inside `azalea-core` (`FixedBitSet`) | A newer nightly than the pin broke upstream. Use the pinned date (`rustup override` is unnecessary if `rust-toolchain.toml` is honoured); bumping the pin is a deliberate dependency-upgrade task. |
| `azalea requires a nightly compiler` | Stable/default toolchain active. Ensure `rustup component add rust-src` isn't masking the override — check `rustc -vV` reports the pinned nightly date inside the repo. |
| Very slow debug runtime | Expected if dependency opt-level was lowered; keep `[profile.dev.package."*"] opt-level = 3`. |
| `link.exe not found` (Windows) | Install Visual Studio Build Tools with the C++ workload. |
