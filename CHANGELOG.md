# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Disconnect during connection attempt:** `ClientBuilder::start()` in
  `connection.rs` is now wrapped in `tokio::select!` with
  `cancel_token.cancelled()`, so clicking Disconnect while azalea is still
  trying to TCP-connect aborts the attempt immediately instead of waiting
  for the ~5 s timeout to expire.
- **Release console window:** `src/main.rs` now carries
  `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` so
  release builds on Windows no longer flash a black console window. Debug
  builds retain the console for diagnostics.

## [1.0.3] - 2026-07-15

### Fixed — P0 (project correctness)

- **u32→i32 溢出 (P0-#1):** added `clamp_to_i32` helper in `src/mcp/mod.rs`;
  every `radius` (and similar) parameter passed into azalea is now explicitly
  clamped from `u32` to `i32` before the cast. `test_radius_clamp_overflow`
  covers `radius = 5_000_000_000_u32` → `i32::MAX`. Affected handlers:
  `get_nearby_blocks` / `get_nearby_entities` / `handle_act` action distance
  parameters.
- **`handle_act` 输入验证 (P0-#2):** new `command_validate::validate_act_action`
  rejects out-of-range coordinates (`y = 9999` → `InvalidParams`,
  `y = -100` → `InvalidParams`) and `attack.entity_id > i32::MAX as u32` before
  the command reaches the bot executor. `test_act_input_validation` covers the
  boundary cases.
- **状态机 skip_equip 转换 (P0-#3 + P1-#6):** `MineBlockOperation` now has a
  `ToolAlreadyInInventory` event. The transition table adds
  `(MovingToTarget, ToolAlreadyInInventory) → Mining`, so when the inventory
  already contains the required tool the bot skips the equip detour. Also
  fixes the `needs_move_to_hotbar = true` path: `tool_type` is preserved
  across the hotbar-move step (was being reset, which produced 11.25s
  hand-mining times instead of the correct 1-2s with iron pickaxe). Regression
  tests: `test_mine_block_skip_equip_when_tool_in_inventory` and
  `test_mine_block_preserves_tool_type_when_hotbar_move_needed`.
- **proptest valid_tools 缺变体 (P0-#4):** `tests/proptest.rs::valid_tools`
  array now contains all 7 `ToolType` variants (Pickaxe, Axe, Shovel, Hoe,
  Sword, Shears, Hand). 100-iteration run with `--test-threads=1` passes with
  zero false positives.
- **rmcp build.rs git config 副作用 (P0-#5):** `patches/rmcp/build.rs` and
  the `build = "build.rs"` line in `patches/rmcp/Cargo.toml` are removed.
  `scripts/install-hooks.sh` added as a no-op stub (documented in README).
  `cargo build` no longer mutates the user's `core.hooksPath`. Verified with
  `cargo update -p rmcp` that crates.io 1.8.0 is used directly with no
  patches required.

### Fixed — P1 (UX / reliability)

- **动态超时 (P1-#10):** `BotCommandSender` no longer stores a `timeout:
  Duration` field; `send_command` reads `SharedState::command_timeout_secs`
  on every call (UI changes take effect immediately). The `as_secs()`
  truncation bug (sub-second timeouts rounded to 0) is fixed by preserving
  the `Duration` value end-to-end instead of converting to `u64` seconds.
  `RealBotClient::goto` uses `sender.timeout()` instead of re-reading the
  config. Tests: `test_send_command_uses_latest_timeout` and the sub-second
  timeout preservation test.
- **ReceiverLease 重试 (P1-#9):** `handle_spawn` now retries
  `ReceiverLease::take` with `yield_now` + `100ms` sleep for up to ~10
  attempts on fast-reconnect paths where the slot is briefly empty. After
  the 100ms budget the executor falls back to a "no executor" warn path
  instead of failing the spawn. Test:
  `test_receiver_lease_take_retries_during_reconnect`.
- **goto 早返回 + notify 兜底 (P1-#11):** `RealBotClient::goto` now races the
  `pathfinder_finished` notify against a 50ms fallback re-check of the bot's
  actual position; if both miss, the original `timeout_dur` path reports
  `PathfindingFailed` as before. Test:
  `test_goto_falls_back_to_position_check`.
- **Disconnect 不提前 set_online(false) (P1-#12):** the Disconnect button
  callback in `src/ui/settings.rs` no longer calls `set_online(false)`
  directly — that flag is owned by the bot ECS (`handle_disconnect`). The
  button just sets `disconnect_requested` and waits. `is_online() == true`
  for up to ~100ms after the click while the ECS tears down.
- **BotError::Offline → -32000 SERVICE_UNAVAILABLE (P1-#13):** the
  `From<BotError> for ErrorData` and `IntoCallToolResult for BotError` impls
  now map `BotError::Offline(_)` to JSON-RPC code `-32000` (not the
  previous `-32603 InternalError`) with
  `data: serde_json::json!({"reason": "bot_disconnected"})`. MCP clients
  can now distinguish "bot is gone" from "input is invalid" and from
  "internal server error". Tests: new
  `test_into_mcp_error_offline_uses_service_unavailable` and the updated
  `test_into_mcp_error_offline`.
- **connect_bot 防御性 join (P1-#14):** `MinecraftApp::connect_bot` now
  extracts a `join_with_timeout(handle, 1s)` helper. When a previous
  `JoinHandle` is still alive on a second Connect, the UI waits up to 1
  second for the old thread to finish (the bot thread already self-joins
  on disconnect; this just covers the edge case). Test:
  `test_connect_bot_joins_old_thread`.
- **Chunk 预检放宽 (P1-#7):** `handle_break_block` no longer depends on
  `partial_instance.chunk_summary` (which is `None` for partial chunks
  that still contain the target block). The precheck now uses
  `block_index.get(&pos).is_some()` as the authoritative signal. Test:
  `test_break_block_loaded_chunk_not_rejected` with a 1-block snapshot.
- **execute_place_block 邻居扫描 (P1-#8):** `execute_place_block` now calls
  `compound_ops::find_standable_neighbor` to pick a standable MoveTo target
  adjacent to the placement position, then issues `use_item_on_block`. The
  bot no longer pathfinds into the block it is about to place (which used
  to fail with "no path" on a wall-adjacent placement). Test:
  `test_place_block_finds_neighbor`.

### Added — CI gate (Phase 1 基础设施)

- **CI 门禁加 test + clippy + fmt:** `.github/workflows/build.yml` and
  `.github/workflows/release.yml` now run a separate `lint` job on
  `ubuntu-latest` that executes `cargo test --locked --all-targets`,
  `cargo clippy --locked --all-targets -- -D warnings`, and
  `cargo fmt --check`. The release workflow's `release` job depends on
  both `build` and `lint` so a failing lint blocks the GitHub Release.
  No more "green CI but locally broken" surprises.

### Added — Harvest Level enforcement (H-1)

- **`block_data::HARVEST_LEVEL: HashMap<&str, u8>`** maps a block's required
  harvest tier: `Wood=0, Stone=1, Iron=2, Diamond=3, Netherite=4`.
  Covers `Stone` / `Cobblestone` (→Iron, 2), `IronOre` / `DeepslateIronOre`
  (→Stone, 1), `DiamondOre` / `DeepslateDiamondOre` (→Iron, 2),
  `Obsidian` / `AncientDebris` (→Diamond, 3), `NetheriteBlock` (→Netherite, 4).
- **`tool_select::find_tool_in_inventory` gains a
  `required_harvest_level: u8` parameter.** Any tool whose own
  `ToolMaterialTier < required` returns `None` (no longer optimistically
  selected). When `None` is returned, the caller surfaces
  `BotError::ToolNotFound { alternatives: ["IronPickaxe"] }` (or the
  appropriate next tier) so the LLM can instruct the player to upgrade.
  Tests: `test_harvest_level_wood_cant_mine_diamond` (asserts `None` +
  alternatives) and `test_harvest_level_diamond_mine_diamond_ore`
  (asserts `Some(DiamondPickaxe)`).

### Changed — `find_standable_neighbor` scans 8 directions (M-4 升级)

- **`compound_ops::find_standable_neighbor`** now scans 8 horizontal
  neighbours (4 cardinal + 4 diagonal) × 3 Y levels, priority
  same-Y → y+1 → y-1. Previously only the 4 cardinal directions were
  considered, which missed the common "next to a wall, only a diagonal
  tile is standable" case. Test:
  `test_find_standable_neighbor_8_directions` and proptest coverage
  (`prop_standable_neighbor_*`).

### Changed — `execute_mine_block` reads through `block_index` (M-12 合规)

- **`src/bot/ops.rs` Step 2** (look up the target block's `block_type` to
  choose a tool) and **Step 10** (verify the block was actually broken)
  now go through `snapshot.block_index.get(&pos)` — the O(1) HashMap
  lookup — rather than `Vec::iter().find()`. With a 5000-block snapshot
  the mine path is now < 1 ms per call. Test:
  `test_mine_block_uses_block_index`.

### Changed — Container slot upper bound unified to 54 (index 53)

- **`src/command_validate.rs` slot upper bound** raised from `50` to `53`
  (0-indexed, matching vanilla chest/large-chest row layout: 9 slots per
  row × 6 rows = 54 total, indices 0..=53). The MCP JSON schema for
  `take_from_container` / `put_into_container` now advertises
  `maximum: 53`. Tests: `test_take_from_container_slot_53_accepted` and
  `test_take_from_container_slot_54_rejected`.

### Added — incremental test coverage

- **`test_command_timeout_responder_alive_but_slow`** in
  `tests/integration.rs` and `test_command_timeout_returns_error` in
  `src/channel.rs::tests` — both assert that a slow but still-alive
  responder produces `BotError::CommandTimeout`, not `Offline`.
- **`test_connect_resets_injections_each_iteration`** in
  `src/bot/connection.rs::tests` — mocks `ClientBuilder::start` for two
  reconnect iterations and asserts all four `INJECTED_*` globals are
  `Some` on the second round.
- **`prop_standable_neighbor_*`** in `tests/proptest.rs` — 1000 random
  snapshots with Y extremes, diagonal-only standable positions, and
  empty snapshots. All run in < 2 s with 0 failures.

### Added — MCP zero-RTT validation

- `handle_drop_item` now rejects `count == 0` at the MCP layer
  (`InvalidParams`) instead of letting the bot executor fail later.
- `handle_send_chat` rejects empty `message` at the MCP layer.
- `handle_act` validates `attack.entity_id ≤ i32::MAX` at the MCP
  layer. Clients get a clear, immediate error rather than a delayed
  bot-side "entity not found".

## [1.0.2] - 2026-07-14

Release notes now include a bilingual (English / 简体中文) download table
with links for every platform and archive format.

### Changed

- `.github/workflows/release.yml` generates a `release-body.md` with English
  and Chinese download tables, then passes it to `action-gh-release` via
  `body_path`.

## [1.0.1] - 2026-07-14

All binaries are now released in two archive formats per platform.

### Changed

- Release workflow (`.github/workflows/release.yml`) now produces:
  - Windows: `.zip` and `.7z`
  - Linux / macOS: `.tar.xz` and `.tar.gz`

## [1.0.0] - 2026-07-14

First stable release. Consolidates all review-fix work from PRs #6–#9 plus the
initial MCP server / UI feature set.

### Added

- GitHub Actions release workflow (`.github/workflows/release.yml`) — builds
  and publishes binaries for Windows / macOS / Linux × x86_64 / aarch64 on
  every `v*` tag push or manual workflow dispatch.

### Added

- **`compound_ops::find_standable_neighbor(snapshot, target) -> Option<BlockPos>`**
  — scans ±1 X/Z × 3 Y levels (priority: same Y, y+1, y-1) for an air block
  with a solid block below, used by `execute_mine_block` to pick a standable
  MoveTo target instead of the block's own position.
- **`WorldSnapshot::block_index: HashMap<BlockPos, usize>`** (`#[serde(skip)]`)
  — O(1) position → block-entry index built by `SnapshotBuilder::build`,
  consumed by `find_obstacle_block`.
- **`state::McpServerStatus` enum** (`Running(SocketAddr)` / `Stdio` /
  `Failed(String)` / `Stopped`) + `SharedState::set_mcp_server_status()` /
  `get_mcp_server_status()` — surfaces real-time MCP server state to the UI
  Status panel.
- **`SharedState::modify_snapshot<F: FnMut(&mut WorldSnapshot)>`** closure API
  (based on `ArcSwap::rcu`) for atomic read-modify-write of the world snapshot.
- **`SharedState::shutdown_token()` / `SharedState::trigger_shutdown()`** —
  graceful shutdown signal for the MCP server (`serve_http` / `serve_stdio`);
  triggered by `MinecraftApp::drop`.
- **`BotState::tick_abort_handles: Arc<Mutex<Vec<AbortHandle>>>`** — tracks
  `handle_tick` `spawn_local` tasks so `handle_disconnect` can abort them.
- **`EditConfig::apply` now returns `Result<(), String>`**, invoking
  `AppConfig::validate()` and refusing empty/invalid configs (setting
  `last_error`).

### Fixed

- **M-2**: 添加 `Event::Packet` 处理器，监听 `ClientboundGamePacket::BlockUpdate` 和 `SectionBlocksUpdate`，通过 `DirtyTracker::mark_block_dirty` 标记脏块。修复快照陈旧问题（之前只有 `ReceiveChunk` 事件触发快照重建，单方块更新被忽略）。
- **M-3**: 由 M-2 自然解决 — `execute_mine_block` 状态机现在能在挖掘完成后观察到方块消失并达到 `Completed`。
- **M-4**: `execute_mine_block` 的 `MoveTo` 目标改为方块的可站立邻居（`find_standable_neighbor` 扫描 ±1 X/Z × 3 Y 层），不再把方块自身位置作为路径终点。修复"到达方块内部后被卡住"问题。
- **M-5**: `render_topdown` 现在按 `(x, z)` 分桶并保留最高 Y 的方块（跳过 air），修复之前忽略 Y 坐标导致低层方块覆盖顶层方块的问题。
- **M-7**: `SnapshotUpdater` 计算 `player_chunk` 与 `chunk_scan_radius`（来自 `AppConfig`），按 Chebyshev 距离过滤脏块扫描范围，避免对远处 chunk 全量扫描。
- **M-8**: HTTP transport + 非 loopback `mcp_address` 时，Settings 面板和 MCP Config 面板显示 TLS 警告（"⚠ No TLS — use trusted network or reverse proxy"）。
- **M-9**: 首次连接（`!was_online`）失败时重试 3 次（2s 间隔，可被 `cancel_token` 打断），超过后才 fail-fast。修复瞬时网络抖动导致需要手动重连的 UX 问题。
- **M-10**: 脏块以单方块粒度直接读取（不再触发整个 chunk 全量扫描）；当脏块所在 chunk 在 `chunk_scan_radius` 内会被全量扫描时，跳过该单块读取以避免重复。
- **M-11**: `handle_act` 的 `perception_radius` 现在读取 `AppConfig::block_perception_radius`（之前硬编码 `16`）。
- **M-12**: `WorldSnapshot` 新增 `block_index: HashMap<BlockPos, usize>`，`find_obstacle_block` 通过 O(1) 索引查找，不再 `Vec::iter().find()` 线性扫描。
- **M-13**: `SharedState` 新增 `McpServerStatus` 枚举（`Running(addr)` / `Stdio` / `Failed(msg)` / `Stopped`），Status 面板显示 MCP 服务器实时状态。
- **S-4**: `BotCommandSender::send_command` 在 responder 被丢弃时返回 `BotError::Offline("bot command responder dropped without reply")`，与真正的超时（`CommandTimeout`）区分开 — responder 永久消失不等同于"还没回复"。
- **S-5**: `send_command` 移除每次调用都执行的 `format!("{:?}", cmd)`，改用 `tracing` 的 `?cmd_for_log` 懒格式化；`format!` 只在超时错误路径保留。
- **S-7**: `logging.rs` 默认 filter 改为 `minecraft_mcp_rs=debug,azalea=warn`（之前是 `info`），方便调试 bot 行为。
- **C-1**: 消除 `CompoundOpExecutor` 在同一命令通道上递归发送导致的死锁 — 改为通过 `&CommandExecutor` 引用直接 `dispatch` 子命令
- **C-2**: 修复重连时 `INJECTED_*` 全局状态被 `handle_disconnect` 清空后未重新设置的问题 — 注入逻辑移入重连 `loop` 内部
- **M-1**: `handle_act` 返回的 `BotResult.success` 现在根据子操作成败派生，不再硬编码为 `true`
- **M-6**: MCP HTTP 服务器绑定地址现在从 `AppConfig.mcp_address` 读取，不再硬编码 `127.0.0.1`
- **(this branch) Snapshot race actually fixed:** `handle_death` /
  `add_player_to_snapshot` / `handle_remove_player` / `handle_update_player`
  now use `SharedState::modify_snapshot` (atomic RCU via `ArcSwap::rcu`).
  Previously the handlers used `read_snapshot().clone()` + `update_snapshot()`,
  which lost updates when `SnapshotUpdater` interleaved at await points
  (the earlier H1 entry described the API as adopted before it actually
  was — the migration is now genuinely done; all `TODO(race)` comments
  removed).
- **(this branch) `Act::Mine` now waits for completion (supersedes H8):**
  `handle_act`'s `ActAction::Mine` branch delegates to
  `CompoundOpExecutor::execute_mine_block(pos, true)`, which selects the
  best tool, walks to the block, mines, and verifies completion. The
  `BotCommandSender` is injected via a new chain
  `ConnectionManager::connect` → `INJECTED_COMMAND_SENDER` →
  `BotState.command_sender` → `CommandExecutor::sender` →
  `CompoundOpExecutor`. When `sender` is `None` (unit tests with a mock
  executor), the handler falls back to the legacy fire-and-forget path
  with a `warn!` log so existing tests stay green. H8's "warn that it's
  fire-and-forget" stopgap is no longer the production behaviour.
- **(this branch) `shutdown_token` lifecycle implemented (compile-blocker
  fix):** `SharedState::shutdown_token() -> CancellationToken` and
  `SharedState::trigger_shutdown()` are now actually defined. The AGENTS.md
  convention was documented but the methods were never implemented,
  causing E0599 on `src/ui/app.rs:210` (which called `trigger_shutdown()`
  from `MinecraftApp::drop`). `serve_http` now uses
  `axum::serve(...).with_graceful_shutdown(async move { token.cancelled().await; })`
  (the `async move` wrapper is required because `CancellationToken::cancelled()`
  borrows the token and is not `'static`); `serve_stdio` uses `tokio::select!`
  racing `shutdown_token.cancelled()` against `running.waiting()` so shutdown
  returns immediately instead of waiting for stdin EOF.
- **H1:** `SharedState::modify_snapshot` adopted by `handle_death` /
  `add_player_to_snapshot` / `handle_remove_player` / `handle_update_player`,
  eliminating the snapshot lost-update race (all `TODO(race)` comments removed).
- **H2:** `BotState::tick_abort_handles` aborts in-flight `handle_tick` tasks
  on `handle_disconnect`, preventing ECS-teardown panics.
- **H3:** `serve_http` uses `axum::serve(...).with_graceful_shutdown(...)`
  for graceful shutdown.
- **H4:** `serve_stdio` races `shutdown_token.cancelled()` against stdin EOF
  via `tokio::select!` so shutdown returns immediately.
- **H5:** MCP thread `JoinHandle` stored in `MinecraftApp`; `Drop` calls
  `trigger_shutdown()` and joins the MCP thread (3s timeout, matching the bot
  thread).
- **H6:** `extract_bearer_token` matches the `Bearer ` scheme case-insensitively
  (`eq_ignore_ascii_case`) per RFC 6750 §2.1; doc comment corrected.
- **H7:** `default_mcp_token()` now generates a random UUID v4 (replacing the
  hardcoded `"minecraft-mcp-rs"`); `EditConfig::apply` rejects empty tokens;
  UI token input masked with `.password(true)`.
- **H8:** `Act::Mine` now returns `success: false` + a `warning` and a message
  flagging mining as fire-and-forget (completion unverified).
- **H9:** `handle_smart_move`'s `Err(e)` branch returns `success: false`
  instead of the misleading `success: true`.
- **H10:** `handle_collect_items` returns `approached` (renamed from
  `collected`) with message "approached N items, pickup unverified".
- **H11:** `take_from_container` / `put_into_container` schema descriptions
  clarify that `count` is currently ignored (whole stack moved); response JSON
  now includes a `warning` field.
- **H14:** `handle_act`'s `perception_radius` now reads
  `AppConfig::block_perception_radius` instead of the hardcoded `16`.
- **H15:** `find_obstacle_block` and 7 nearby-filter sites use
  `i32::abs_diff` instead of `(a-b).abs()`, eliminating `i32::MIN`/`MAX`
  overflow panic risk.
- **H16:** `logging.rs` Mutex locks use `.unwrap_or_else(|e| e.into_inner())`
  for poisoning recovery, consistent with `state.rs` / `channel.rs` /
  `events.rs`.
- **L1:** `handle_spawn` reordered to call `set_bot_ecs(...)` before
  `set_online(true)`, ensuring `bot_ecs` is ready to write `AppExit::Success`
  when disconnect fires.
- **M11:** `handle_disconnect` calls `set_container_handle(None)` to clear
  stale container handles on reconnect.
- **M13:** Bearer token comparison now uses constant-time
  `constant_time_token_eq` (byte-wise OR accumulator), closing the timing
  side-channel.
- **R1:** `get_nearby_blocks` / `get_nearby_entities` are now parameterised
  with `radius` (and an optional `filter_type` for blocks), restoring the
  original API contract instead of the previous hardcoded `radius=10`.
- **R2:** `break_block(use_best_tool=true)` now delegates to the full
  compound mine flow by sending `BotCommand::Act(ActAction::Mine)`, matching
  the behaviour of `act(Mine)`.
- **R3:** Introduced `BotCommand::UseItemWithSlot(u8)` so the bot executor
  can atomically switch hotbar slot and use the item, eliminating the race
  where concurrent `use_item` calls interleaved their `SwitchHotbarSlot`
  and `UseItem` commands.
- **R4:** `WalkDirection.distance` is validated to `1..=1000` and the MCP
  JSON schema now advertises `"maximum": 1000`.
- **R5:** `AttackEntity.entity_id` is validated to `<= i32::MAX` before the
  `u32 -> i32` cast used by azalea lookups.
- **R6:** `query_inventory` and `execute_place_block` now bounds-check
  values before casting to `u8`, returning clear errors instead of silently
  truncating.
- **R7:** `encode_png` propagates PNG encoder errors via `BotError::Internal`
  instead of panicking with `.expect()`.
- **R8:** `AppConfig::validate()` now rejects `mc_port == 0` and
  `mcp_port == 0`.
- **R9:** Integration test `test_all_bot_command_variants_exist_no_craft_item`
  now enumerates all 33 `BotCommand` variants (including the new
  `UseItemWithSlot`) and all 6 `ActAction` sub-variants.
- **R10:** Previously-unused `BotError` variants are now returned from the
  relevant handlers: `ChunkNotLoaded` from `handle_break_block`,
  `TooFar` from `handle_attack_entity`, and `InventoryFull` from
  `handle_take_from_container`. `ConnectionFailed` remains a typed error
  for future connection-layer use.
- **R11:** The four `OnceLock` injection globals in `bot/events.rs`
  (`INJECTED_SHARED_STATE`, `INJECTED_COMMAND_RECEIVER`, `INJECTED_EGUI_CTX`,
  `INJECTED_COMMAND_SENDER`) are now `Mutex<Option<T>>` and are cleared in
  `handle_disconnect`, so reconnects and tests get fresh injection state.
- **R12:** `INJECTED_EGUI_CTX` is now populated with the live egui
  `Context` from `eframe::Frame`, allowing bot event handlers to request
  immediate UI repaints.

### Added

- `BotCommand::UseItemWithSlot(u8)` — atomic switch-hotbar-slot-and-use-item
  command.
- `src/utils.rs` common utility module, currently exporting
  `to_snake_case` used by the command executor and snapshot updater.

### Changed

- Replaced hand-written `impl rmcp::schemars::JsonSchema` in the seven MCP
  tool modules (`tools_query`, `tools_movement`, `tools_block`, `tools_item`,
  `tools_container`, `tools_combat`, `tools_chat`) with
  `#[derive(Deserialize, rmcp::schemars::JsonSchema)]`. This removes a source
  of schema drift when new fields are added.
- `block_data::find_best_tool_in_inventory` is now `#[deprecated]` and
  delegates to `tool_select::find_tool_in_inventory`; callers and tests were
  updated to use the canonical implementation.

### Removed

- Dead `act_tool()` builder function and its `test_act_tool_builder` unit
  test. Production tools are registered via the `#[tool]` macro + derive.

### Known Issues

- **C1 (deferred):** `From<BotError> for ErrorData` is still dead code — MCP
  tool handlers continue to return `String`, so clients cannot receive
  structured error codes. Fixing requires re-signing all `#[tool]` handlers to
  return `Result<CallToolResult, ErrorData>` (larger refactor, out of scope for
  this branch). See `review-report.md` C1.
- **H12 (deferred):** `RealBotClient::goto` still uses a 10Hz busy-poll loop
  with a hardcoded 30s timeout decoupled from `AppConfig::command_timeout_secs`.
  Fix requires azalea pathfinder completion-event integration or a wider
  polling refactor, out of scope for this branch. See `review-report.md` H12.
- **H13 (deferred):** Dirty-chunk full scans (98304 blocks/chunk) in
  `build_snapshot_inner` still lack spatial indexing / Vec preallocation /
  `&'static str` block-name caching; performance refactor out of scope for
  this branch. See `review-report.md` H13.
- **tick_abort_handles growth (deferred):** `tick_abort_handles` in
  `BotState` grows unbounded over a single long session — tick tasks spawned
  via `spawn_local` push an `AbortHandle` that is never reaped after the task
  completes; the `Vec` is only drained on disconnect. At ~2 spawns/sec this
  accumulates roughly 1–2 MB/hour. Suggested fix: migrate to a `JoinSet` or
  periodically `retain(|h| !h.is_finished())`.

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
