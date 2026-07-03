# Getting Started

This guide walks through building, running, and testing `minecraft-mcp-rs` — a
Minecraft bot controlled via the Model Context Protocol (MCP).

## Prerequisites

- [Rust nightly](https://rustup.rs/) — pinned in `rust-toolchain.toml`
  (edition 2024; azalea 0.15.1's build script requires nightly)
- A **Minecraft Java Edition 1.21.11** server (local or remote)
- An **MCP client** — Claude Desktop, Cursor, or any MCP-compatible LLM host

## Build

```bash
cargo build
```

## Run

```bash
cargo run
```

This starts both the MCP server and the egui desktop UI. Choose the MCP
transport in the Settings panel:

- **stdio** — the MCP server listens on stdin/stdout (default for Claude
  Desktop / Cursor).
- **HTTP** — the MCP server binds to `127.0.0.1` only; set the port and
  Bearer token (defaults to the project name `minecraft-mcp-rs`). The MCP
  Config panel generates the matching JSON config for copying into your MCP
  client.

By default the bot tries to connect to `127.0.0.1:25565` as `AI_Bot`. Tweak
settings in the UI panel or via environment before startup (see
[Configuration](./configuration)).

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
