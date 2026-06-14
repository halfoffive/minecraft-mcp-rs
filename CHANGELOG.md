# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Settings panel now fully editable:** all fields use `TextEdit::singleline`
  for strings and `DragValue` sliders for numeric values (with appropriate
  range limits).  Edits are persisted to `SharedState` on Connect.
- **Connect / Disconnect buttons are wired up:** Connect spawns the bot
  connection on a dedicated OS thread (bypasses azalea `LocalSet` `!Send`
  limitation).  Disconnect sets `bot_online = false`.

### Fixed

- **30+ Clippy warnings** resolved across 14 source files:
  - Removed unused imports (`BlockEntry`, `GameMode`, `SelfPlayer`, `WorldSnapshot`)
  - Replaced `map_or(false, …)` with `is_some_and(…)` (2 occurrences)
  - Collapsed redundant `if` guards / nested `if` statements (7 occurrences)
  - Replaced manual range checks with `RangeInclusive::contains`
  - Converted `useless_vec!` to arrays (3 occurrences)
  - Added `#[allow(deprecated)]` on azalea `Block` alias usages
  - Added `#![allow(dead_code)]` on architecture-scaffolding modules
  - Removed duplicate `mod` declarations in `main.rs`
  - Fixed `#[allow(clippy::field_reassign_with_default)]` on test helpers
  - Added `let _ =` on `unused_must_use`

## [0.1.0] — 2025-03-27

### Added

- **MCP server** (rmcp, stdio transport) exposing 25+ tools:
  - **Query:** `get_self_info`, `get_inventory`, `get_nearby_blocks`,
    `get_nearby_entities`, `get_chunk_summary`, `is_connected`
  - **Movement:** `move_to`, `walk_direction`, `jump`, `teleport`
  - **Block:** `break_block`, `place_block`, `use_item_on_block`
  - **Item:** `drop_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`
  - **Container:** `open_container`, `take_from_container`,
    `put_into_container`, `close_container`
  - **Combat:** `attack_entity`, `shield_block`
  - **Chat:** `send_chat`, `execute_command`, `set_gamemode`
- **Minecraft bot** (azalea) with connection lifecycle:
  - Connect, disconnect, auto-reconnect with exponential backoff
  - Event handling for position updates, chunk loads, chat messages
  - Command execution for all supported actions
  - Snapshot updater that periodically captures world state
- **Thread-safe shared state** (`ArcSwap` for snapshots, `RwLock` for config,
  `AtomicBool` for online flag, `Mutex` for containers and chat)
- **Block data tables** with mining time calculations and best-tool selection
- **Coordinate validation** and command pre-checks
- **Compound operation state machines** (mine-and-collect pipeline)
- **Desktop UI** (egui/eframe):
  - Status panel with live command counters and connection state
  - Settings panel for all configurable parameters
- **Logging** via `tracing` to stderr only (stdout reserved for MCP transport)
- **Comprehensive tests:**
  - Unit tests in each source module
  - Mock-based integration tests (no real Minecraft server required)
  - Property-based tests for block data and coordinate validation
