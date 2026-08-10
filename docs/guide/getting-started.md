# Getting Started

This guide walks through building, running, and testing `minecraft-mcp-rs` — a
Minecraft bot controlled via the Model Context Protocol (MCP).

## Prerequisites

- [Rust nightly](https://rustup.rs/) — pinned in `rust-toolchain.toml`
  (edition 2024; azalea 0.15.1's build script requires nightly)
- A **Minecraft Java Edition 1.21.11** server (local or remote) — **the only supported version**
- An **MCP client** — Claude Desktop, Cursor, or any MCP-compatible LLM host

> Only **Minecraft Java Edition 1.21.11** is supported — no other server version works. If your server runs a different version, install the matching `minecraft-mcp-rs` release (see [npm install](../npm#version-compatibility)).

| Minecraft server version | minecraft-mcp-rs version |
|--------------------------|--------------------------|
| 1.21.11                  | 1.1.3                    |

## Build

```bash
cargo build
```

## Run

```bash
cargo run -- --gui
```

`cargo run` with no arguments prints the usage and exits; the desktop UI
starts with `--gui`. Choose the MCP transport in the Settings panel:

- **stdio** — the MCP server listens on stdin/stdout (default for Claude
  Desktop / Cursor). `--stdio` alone (without `--gui`) implies headless
  mode: no window, auto-connect, and the process exits when the MCP
  transport closes.
- **HTTP** — the MCP server binds to `127.0.0.1` only; set the port and an
  optional Bearer token (auth is off by default; the token defaults to a
  random UUID v4). The MCP Config panel generates the matching JSON config
  for copying into your MCP client.

By default the bot tries to connect to `127.0.0.1:25565` as `AI_Bot`. Tweak
settings in the UI panel or via environment before startup (see
[Configuration](../config)).

## Test

```bash
cargo test                         # all tests
cargo test --lib                   # unit tests only
cargo test --test integration      # mock-based integration tests
cargo test --test proptest         # property-based tests
```

Unit tests live at the bottom of each source file in
`#[cfg(test)] mod tests`. Integration tests in `tests/integration.rs` use
mocks (no real MC server), and property tests in `tests/proptest.rs` use the
`proptest` crate.
