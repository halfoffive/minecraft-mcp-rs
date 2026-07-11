# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **UI internationalization (i18n):** desktop UI now supports English and
  Simplified Chinese, switchable at runtime via a Language dropdown in the
  Settings panel (takes effect next frame, no reconnect needed). Translation
  strings live in a functional one-file-per-language module
  (`src/ui/i18n/{en,zh_cn}.rs`) with a thread-safe `tr()` lookup.
- **CJK system font auto-loading:** on startup the app probes the platform's
  default CJK font (Windows `msyh.ttc`, macOS `PingFang.ttc`, Linux
  Noto/WenQuanYi) and injects it into egui `FontDefinitions` so Chinese text
  renders without manual font setup. Falls back to the default font with a
  `warn` log if none is found.
- **`AppConfig::language` field** (`Language::En` default, `#[serde(default)]`
  for backward-compatible deserialization of older config files).
- **Cross-platform multi-architecture CI:** `.github/workflows/build.yml`
  builds the release binary for Windows / macOS / Linux × x86_64 / aarch64
  (native ARM runners) and uploads per-target artifacts.
- **VitePress documentation site** under `docs/` with English + Simplified
  Chinese locales (mirrors the vuejs/vitepress config split pattern).
- **GitHub Pages deployment workflow** `.github/workflows/deploy-docs.yml`
  builds the VitePress site with Node 24 and deploys via
  `actions/deploy-pages@v5`.

### Changed

- **GitHub Actions runtime bump:** workflows now use Node.js 24 and the
  latest Node-24-compatible official actions — `actions/checkout@v6`,
  `actions/setup-node@v6`, `actions/upload-artifact@v7`,
  `actions/upload-pages-artifact@v5`, and `actions/deploy-pages@v5`.
- **CI build matrix update:** dropped the `x86_64-apple-darwin` / `macos-13`
  target due to long GitHub-hosted runner queues and deprecation, and moved
  `aarch64-apple-darwin` to `macos-latest` (Apple Silicon).
- All hardcoded English strings in `app.rs`, `settings.rs`, `status.rs`, and
  `mcp_config.rs` now route through `i18n::tr()`. MCP tool descriptions and
  JSON field names remain English (external API contract).
- **GitHub Actions path filtering:**
  - `.github/workflows/deploy-docs.yml` now only triggers on `push` when
    `docs/**` or the workflow itself changes.
  - `.github/workflows/build.yml` now only triggers on `push` and
    `pull_request` when `src/**`, `tests/**`, `patches/**`, `Cargo.toml`,
    `Cargo.lock`, `rust-toolchain.toml`, or the workflow itself changes.
- **Bilingual README:** `README.md` main content is now expanded to English
  and Simplified Chinese, with a Chinese translation immediately following
  each major English paragraph.

- Remote MCP HTTP server (`transport-streamable-http-server` feature):
  binds to `127.0.0.1` only, Bearer-token authenticated, port/token configurable
  in the UI. Default token is the project name `minecraft-mcp-rs`.
- MCP transport selector in the Settings panel (`Stdio` / `Http`) and a live
  MCP Config panel that generates copyable JSON for the selected transport.
- AI vision for multimodal models — `get_world_view` renders a top-down PNG of
  nearby blocks, base64-encodes it, and returns it to the LLM.
- New MCP tools:
  - `get_chat_history` — recent chat messages.
  - `get_server_info` — current world / server flags, including whether
    commands are enabled.
  - `get_world_view` — top-down visual snapshot of surroundings.
  - `collect_items` — pick up nearby dropped items.
  - `smart_move` — pathfind to a coordinate, auto-jump over 1-block gaps,
    stop and report when blocked by higher obstacles.
  - `fly_to` — creative-mode flight to a coordinate, stopping on obstruction.
  - `act` — unified action tool that can move, smart-move, fly, mine, attack,
    or collect items, then returns an environment snapshot (nearby blocks,
    entities, and self info) so the model can call it repeatedly.
- New `ActAction` / `ActResult` types in `types.rs` to drive the `act` tool.
- `WorldSnapshot.commands_enabled` flag surfaced by `get_server_info`.
- `SharedState` ECS handle storage so `request_disconnect` can write
  `AppExit::Success` and force a clean bot shutdown.
- Local dependency patches (ignored by git) to resolve upstream conflicts:
  - `patches/rmcp` removes the `rand` dependency from rmcp's HTTP feature,
    avoiding the `rand_core` version clash with azalea 0.15.1.
  - `patches/rsa` fixes `pkcs8 0.11.0` compatibility for `rsa 0.10.0-rc.13`.
- `SharedState::last_error` field for surfacing connection errors to the UI.
- MCP Config panel in the desktop UI — displays copyable JSON config for MCP clients.
- `tokio-util` dependency for `CancellationToken`-based disconnect signaling.

### Changed

- Default MCP transport is now `Http` so remote clients can connect without
  extra plumbing.
- Settings panel gained `mcp_transport`, `mcp_address`, `mcp_port`, and
  `mcp_token` fields.
- UI clipboards use the egui 0.34.3 `from_id_salt` API.
- Replaced unstable `std::mem::variant_count` in tests with a stable
  `all_bot_commands().len()` check.

### Fixed

- **`handle_disconnect` now aborts the command executor properly:** previously
  `handle_disconnect` called `handle.abort()` but never `await`ed the handle,
  so the `ReceiverLease` drop guard (which returns the command receiver to the
  slot for the next reconnect) might not run before a prompt reconnect. Now
  `handle.await` ensures the lease is fully released.
- **`handle_spawn` now `await`s the aborted previous executor:** without the
  await, a second `Spawn` event (e.g. respawn after death) could race the
  abort and find the receiver slot still empty, leaving the bot online but
  unable to process any commands. `handle_spawn` is now `async`.
- **`goto` stale-permit false success fixed:** the `notify_one()` drain was
  only on the fast path (target already reached). A stale permit from a tick
  between two `goto` calls could make the slow path return `Ok(())`
  immediately even though the new target was unreached. The drain now runs
  before both paths.
- **`handle_disconnect` clears `container_handle`:** without this, a stale
  handle would make `has_container_open()` return true after reconnect,
  causing `open_container` to incorrectly report `ContainerAlreadyOpen`.
- **`handle_open_container` error priority:** `is_online()` check moved
  before `check_container_not_open()` so a stale container-state from a
  disconnected session does not shadow the more fundamental offline error.
- **`chest`/`trapped_chest` tool mapping:** moved from Pickaxe to Axe (they
  are wooden containers; only `ender_chest` stays on Pickaxe as it is made
  of obsidian).
- **4 stale `tools_container` unit tests fixed:** tests asserting
  `InvalidParams("No container is currently open")` while offline now
  correctly assert `Offline` (or call `make_online` first to reach the
  container check).
- **`test_goto_notify_clone_shares_state` strengthened:** was a sync test
  that never `await`ed the waiter and used `notify_waiters()` (inconsistent
  with production `notify_one()`). Now a proper `#[tokio::test]` that
  verifies cross-Arc wake-up.
- **Removed dead `simulate_container_open` test stub.**
- **`WorldSnapshot::default()` now delegates to `SelfPlayer::default()`**
  instead of hand-constructing the same field values.
- **`act` tool now validates action parameters:** coordinates are
  validated via `validate_block_pos` and `CollectItems` radius is range
  checked (1-1024) before dispatch, matching the standalone tool handlers.
- **Container `take`/`put` now enforce slot (0-53) and count (1-64)
  limits** in the MCP handler, not just in the dead `validate_command`.
- **`drop_item` now enforces count ≤ 64** (max stack size).
- **`tools_chat` tests:** replaced fragile `!contains("Error")` assertions
  with proper `is_ok()` checks.
- **`RunStats` command counters now incremented:** the
  `commands_processed`/`succeeded`/`failed` atomics are updated in the
  command executor loop, so the UI Status panel shows real data.
- **`egui_ctx` now passed to the bot connection thread:** event-driven
  UI repaints (connect, disconnect, chat, death) now work instead of
  falling back to 1-second polling.
- **`server.rs` doc comments updated:** rmcp version (1.8.0), tool count
  (30+), and removed stale schemars v0.8 mismatch note.

- **MCP tool errors are now standard MCP errors:** all `#[tool]` handlers and
  their underlying functions return `Result<T, BotError>`, so the existing
  `From<BotError> for ErrorData` conversion is no longer dead code. Clients now
  receive proper JSON-RPC error objects (`code`/`message`/`data`) instead of
  `{"success":false,"error":"..."}` payloads.
- **`goto` no longer busy-loops:** `BotActions::goto` waits event-driven on a
  `tokio::sync::Notify` instead of polling `is_goto_target_reached()` every
  100ms, and uses `AppConfig::command_timeout_secs` instead of a hard-coded
  30s timeout.
- **Dirty chunk updates no longer scan the whole block list:** `DirtyTracker`
  and `SnapshotBuilder` now maintain a chunk → block positions spatial index,
  so only blocks inside affected chunks are examined when rebuilding the world
  snapshot.
- **Tick task handle leak fixed:** `BotState` tracks per-tick snapshot tasks in
  a `tokio::task::JoinSet`, reclaims completed handles on each tick, and aborts
  + clears them on disconnect, preventing ~1-2 MB/hour growth in long sessions.
- **CI build failure:** `patches/rmcp/` and `patches/rsa/` are now tracked in
  git and included in the repository. Previously they were ignored, causing
  GitHub Actions `cargo build --release --locked` to fail with
  "failed to load source for dependency `rmcp`" because the `[patch.crates-io]`
  path dependencies did not exist in the CI checkout.
- Disconnect now works reliably: `request_disconnect` writes `AppExit::Success`
  into the bot ECS, causing `ClientBuilder::start()` to return and the connect
  loop to exit.

### Changed

- Upgraded azalea from 0.16 to 0.15.1 for Minecraft 1.21.11 compatibility (was 26.1).
- Upgraded eframe/egui from 0.31 to 0.34.3.
- Upgraded schemars from 0.8 to 1.0.3.
- Upgraded all other dependencies to latest compatible versions (tokio 1.50, serde 1.0.228, etc.).
- Kept Rust nightly toolchain (azalea 0.15.1's build script requires nightly; stable is incompatible with MC 1.21.11 support).
- Migrated azalea APIs: `Client::exit()` → ECS `AppExit`, `WorldHolder` → `InstanceHolder`, etc.
- Migrated egui APIs: `App::update` split into `logic` + `ui`; clipboard API updated.
- Connection failures now stop retrying instead of infinite reconnect loops.

### Fixed

- Minecraft 1.21.11 connection failures (azalea 0.16 used the wrong protocol version).
- Window close hanging — `Drop::join` now has a 3-second timeout.
- Reconnect sleep no longer blocks disconnect — `CancellationToken` allows instant cancellation.
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
