# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `calculate_mine_time` now applies the 5× wrong-tool penalty when mining tool-required blocks with an empty hand (e.g. stone).
- MCP tools `use_item_on_block`, `walk_direction`, and `shield_block` now correctly pass their parameters (`item_slot`, `distance`, `blocking`) all the way to the bot executor instead of silently dropping them.
- `get_inventory` now returns the full player inventory from the world snapshot instead of a placeholder stub.
- Mutex lock poisoning no longer cascades crashes: `channel.rs` and `bot/events.rs` now recover from poisoned locks using `.unwrap_or_else(|e| e.into_inner())`.
- `execute_equip_tool` no longer forces a switch to hotbar slot 0 when equipping an empty hand and no tool is found.

### Fixed — P0 (project was non-functional)

- **Command executor wired up:** `Event::Spawn` now starts a
  `CommandExecutor` task via `spawn_local`, leasing the command receiver
  from a shared slot. Previously the executor was never started, so every
  MCP action tool timed out after 30 seconds.
- **`handle_execute_command` double-`/` bug:** the MCP layer already
  normalises the leading `/`, so the executor no longer re-prepends it
  (was sending `//command` to chat, which Minecraft ignores).
- **`handle_place_block` now selects the hotbar slot:** parses the
  `slot:N` prefix from `block_type` and calls `set_selected_hotbar_slot`
  before right-clicking. Previously the slot was ignored.
- **`handle_query_inventory` reads the live inventory:** uses
  `Client::menu()` + `Menu::try_as_player()` instead of returning a
  hardcoded `[]` (which broke all tool-dependent compound ops with
  misleading `ToolNotFound`).

### Fixed — P1 (crash/overflow risks)

- **`Duration::from_secs_f64(INFINITY)` panic guard:** mining unbreakable
  blocks (e.g. bedrock) now returns `BotError::MiningInterrupted` instead
  of panicking the bot thread.
- **`i32::MIN.abs()` overflow guard:** `validate_coordinates` uses
  explicit range checks instead of `.abs()`, avoiding the debug panic /
  release wrap that let `i32::MIN` slip past validation.

### Added — real implementations replacing stubs

- **`open_container`:** async; awaits `Client::open_container_at` and
  stores the `ContainerHandle` in `SharedState` so subsequent container
  commands can borrow it.
- **`take_from_container` / `put_into_container`:** use the stored
  handle's `shift_click` to move stacks (best-effort; `count` is a hint).
- **`equip_tool`:** queries the live inventory, finds the requested tool
  type, and switches to its hotbar slot. Returns `ToolNotFound` when
  absent; `Internal` when the tool is only in the main inventory.
- **`drop_item`:** issues `ThrowClick` on the player inventory menu.
- **`held_item_slot`:** read via `Client::selected_hotbar_slot()` in both
  snapshot builders (was hardcoded to 0).
- **`handle_add_player`:** reads the player entity's live `Position` and
  `MinecraftEntityId` when available.

### Changed — correctness

- **`BotError::InvalidParams`:** new variant mapping to MCP
  `INVALID_PARAMS`; `validate_command` uses it instead of `Internal` so
  clients see the right error code for input errors.
- **Configurable command timeout:** `BotCommandSender::with_timeout`
  honours `AppConfig::command_timeout_secs` instead of hardcoding 30s.
- **`handle_tick` TOCTOU fix:** check-and-set `last_snapshot_time` under
  a single lock to prevent concurrent snapshot builders.
- **`build_and_update_snapshot` early lock release:** `dirty_tracker` is
  released immediately after `take_dirty_sets` so `handle_receive_chunk`
  isn't blocked during world reads.
- **Mutex poisoning recovery:** all `SharedState` locks use
  `unwrap_or_else(|e| e.into_inner())` instead of panicking the app.
- **`execute_place_block` slot fix:** rejects slot >= 9 with a clear
  error instead of letting the executor reject it later.
- **`QueryNearbyEntities` radius cap:** 1..=1024 (prevents `u32 -> i32`
  wrap that silently returned empty results).
- **`handle_set_game_mode` honesty:** message now flags the OP
  requirement instead of asserting success.
- **`handle_use_item` slot switching:** sends `SwitchHotbarSlot` before
  `UseItem` when `item_slot` is provided.

### Changed — UI/lifecycle

- **Connect guard:** `try_begin_connecting` prevents double-spawn when
  the user clicks Connect while a previous attempt is in progress.
- **Real Disconnect:** the Disconnect button calls `request_disconnect`;
  the reconnect loop checks this flag and stops retrying.
- **JoinHandle management:** `MinecraftApp` holds the bot thread handle;
  `Drop` calls `request_disconnect` and joins the thread for clean exit.
- **Reduced repaint frequency:** 10 FPS fallback -> 1 FPS fallback;
  event-driven `request_repaint` covers state changes.
- **`status.rs` lock scope:** `RwLockReadGuard` for `RunStats` is
  dropped immediately after reading `connected_since`; re-acquired only
  for the Command Stats section.

### Changed — architecture

- **Unified `BlockPos`/`ToolType`/`MaterialTier`:** `error.rs` re-exports
  from `types.rs` instead of duplicating with incompatible variants.
  `ToolType` now has all 7 variants (Pickaxe, Axe, Shovel, Hoe, Sword,
  Shears, Hand). Eliminated the lossy `to_error_*` bridge helpers.
- **Single `calculate_mine_time`:** deleted the dead `block_data` version
  (without the 1.5x factor); the canonical `mining_calc` version is used
  everywhere.
- **`MATERIAL_PRIORITY` ordering:** updated to
  `[Netherite, Diamond, Iron, Stone, Gold, Wood]` — the reverse of the
  `Ord` derive, so Gold ranks above Wood (same mining level, higher
  speed).
- **Unified snapshot builder:** `handle_tick` delegates to
  `SnapshotUpdater::update_from_tick` instead of duplicating the logic
  inline. Deleted ~100 lines of duplicate code from `events.rs`.
- **Removed `#![allow(dead_code)]`** from `bot/commands.rs` and
  `block_data.rs` (no longer needed after wiring up the executor).
- **Removed redundant `rmcp`** from `[dev-dependencies]` in `Cargo.toml`.
- **`azalea-inventory`** added as a direct dependency for `ThrowClick`.


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
