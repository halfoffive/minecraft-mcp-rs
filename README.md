<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://img.shields.io/badge/status-alpha-orange?style=flat-square">
  <img alt="status: alpha" src="https://img.shields.io/badge/status-alpha-orange?style=flat-square">
</picture>
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://img.shields.io/badge/rust-nightly-blue?style=flat-square">
  <img alt="rust: nightly" src="https://img.shields.io/badge/rust-nightly-blue?style=flat-square">
</picture>
<br>

# minecraft-mcp-rs

**A Minecraft bot controlled via the Model Context Protocol (MCP).**

This project bridges an LLM client (Claude Desktop, Cursor, etc.) to a live
Minecraft game world. A Rust bot (backed by the [azalea] library) connects to
your Minecraft server and exposes its abilities — moving, mining, building,
inventory management, combat, and chatting — as MCP tools over stdio transport.

The bot targets **Minecraft Java Edition 1.21.11** (via azalea 0.15.1).

[azalea]: https://github.com/azalea-rs/azalea

## Features

- **25+ MCP tools** organized into 7 domains
- **Live world state** — the bot periodically snapshots its surroundings into a
  thread-safe `SharedState` readable by all tools
- **Desktop UI** (egui/eframe) — status panel with live stats, settings panel
  to configure connection and bot parameters, and an MCP Config panel that
  shows a copyable JSON config (with the executable's absolute path) for
  Claude Desktop / Cursor
- **Auto-reconnect** — exponential backoff on disconnect; the command executor
  is cleanly aborted and re-started on each reconnect via a `ReceiverLease`
- **Compound operations** — higher-level state machines (mine-and-collect)
  built on primitive commands
- **Thread-safe by design** — lock-free snapshots via `ArcSwap`, atomic flags,
  `RwLock` for config, `Mutex` for container handles
- **Dirty-region snapshot updates** — only changed blocks/chunks are
  recomputed between polling intervals
- **Configurable command timeout** — `command_timeout_secs` is honoured by
  the command channel (not just a UI field)
- **Honest error reporting** — `BotError::InvalidParams` maps to MCP
  `INVALID_PARAMS`; unbreakable blocks return `MiningInterrupted` instead of
  panicking; `set_game_mode` flags the OP requirement

## Tool Categories

| Category | Tools |
|----------|-------|
| **Query** | `get_self_info`, `get_inventory`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected` |
| **Movement** | `move_to`, `walk_direction`, `jump`, `teleport` |
| **Block** | `break_block`, `place_block`, `use_item_on_block` |
| **Item** | `drop_item`, `equip_tool`, `switch_hotbar_slot`, `use_item` |
| **Container** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **Combat** | `attack_entity`, `shield_block` |
| **Chat** | `send_chat`, `execute_command`, `set_gamemode` |

## Quick Start

### Prerequisites

- [Rust nightly](https://rustup.rs/) (pinned in `rust-toolchain.toml`, edition 2024; azalea 0.15.1 requires nightly)
- A Minecraft Java Edition 1.21.11 server (local or remote)
- An MCP client (Claude Desktop, Cursor, or any MCP-compatible LLM host)

### Build

```bash
cargo build
```

### Run

```bash
cargo run
```

This starts both the MCP server (on stdio) and the egui desktop UI. The MCP
server listens on stdin/stdout — connect your MCP client accordingly.

By default the bot tries to connect to `127.0.0.1:25565` as `AI_Bot`. Tweak
settings in the UI panel or via environment before startup (see Configuration).

### Test

```bash
cargo test                # all tests
cargo test --lib          # unit tests only
cargo test --test integration  # mock-based integration tests
cargo test --test proptest     # property-based tests
```

## Configuration

All settings have sensible defaults and can be changed at runtime through the
egui settings panel (fully editable — text inputs for strings, DragValue
sliders for numeric fields). After editing, click **Connect** to apply the
settings and spawn the bot connection on a dedicated background thread.

| Field | Default | Description |
|-------|---------|-------------|
| `mc_address` | `127.0.0.1` | Minecraft server address |
| `mc_port` | `25565` | Minecraft server port |
| `ai_username` | `AI_Bot` | Bot in-game username |
| `chunk_scan_radius` | `8` | Chunks to scan (1–16) |
| `block_perception_radius` | `32` | Block awareness range (8–64) |
| `snapshot_interval_ms` | `500` | World snapshot interval |
| `reconnect_initial_delay_ms` | `5000` | Initial reconnect backoff |
| `reconnect_max_delay_ms` | `60000` | Maximum reconnect backoff |
| `command_timeout_secs` | `30` | Bot command timeout |

## Architecture

```
┌──────────────────────────────────────────────────┐
│                   egui Desktop UI               │
│  ┌──────────┐  ┌──────────┐                     │
│  │  Status  │  │ Settings │                     │
│  └────┬─────┘  └────┬─────┘                     │
│       └──────┬───────┘                          │
│              │ reads/writes                      │
│              ▼                                   │
│       ┌──────────┐                               │
│       │SharedState│  (ArcSwap + RwLock + Atomics)│
│       └────┬─────┘                               │
└────────────┼─────────────────────────────────────┘
             │ reads
┌────────────┼─────────────────────────────────────┐
│   MCP Server (rmcp, stdio transport)            │
│  ┌──────────┐   ┌───────────────────────────┐   │
│  │  Router  │──▶│ tools_query/movement/... │   │
│  └────┬─────┘   └───────────────────────────┘   │
│       │ sends BotCommand (tokio mpsc + oneshot)  │
│       ▼                                           │
│  ┌──────────┐                                    │
│  │ BotEngine│ (azalea client + bevy_ecs)         │
│  └──────────┘                                    │
└──────────────────────────────────────────────────┘
```

The bot runs on a background OS thread with its own tokio runtime. The UI runs on
the main thread. They communicate through `Arc<SharedState>` (lock-free reads)
and a `BotCommand` channel (tokio mpsc + oneshot for response).

## Project Structure

```
src/
  types.rs            — Shared data types (BlockPos, BotCommand, WorldSnapshot, …)
  error.rs            — BotError enum (actionable variants for AI agents)
  config.rs           — AppConfig + RunStats (atomic counters)
  state.rs            — SharedState thread-safe hub
  block_data.rs       — Block/tool/material tables
  mining_calc.rs      — Mining time formulas
  command_validate.rs — Coordinate validation
  snapshot.rs         — World snapshot + dirty-region tracking
  tool_select.rs      — Best-tool selection
  compound_ops.rs     — Multi-step operation state machines
  channel.rs          — mpsc/oneshot command channel
  logging.rs          — tracing-subscriber (stderr only)
  bot/                — Bot lifecycle, events, commands, ops
  mcp/                — MCP server + 7 tool modules
  ui/                 — egui app shell, settings, status
tests/
  integration.rs      — Mock-based end-to-end tests
  proptest.rs         — Property-based tests
```

## Development

### Logging

All log output goes to **stderr** only — stdout is reserved for MCP JSON-RPC
transport. Default filter: `minecraft_mcp_rs=debug, azalea=warn`.

### Testing Conventions

- Unit tests live at the bottom of each source file in `#[cfg(test)] mod tests`
- Integration tests in `tests/integration.rs` use mocks (no real MC server)
- Property tests in `tests/proptest.rs` use the `proptest` crate

## License

MIT
