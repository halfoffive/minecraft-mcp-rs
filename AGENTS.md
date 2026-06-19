# minecraft-mcp-rs — MCP server that controls a Minecraft bot

An MCP (Model Context Protocol) server backed by an actual Minecraft bot via the azalea library. Exposes bot capabilities (movement, block manipulation, inventory, combat, chat) as MCP tools consumable by LLM clients. Ships with an egui desktop UI for status and settings.

- **Stack:** Rust nightly, azalea (Minecraft bot), rmcp (MCP server), egui/eframe (desktop UI), tokio (async runtime), bevy_ecs (azalea's ECS), proptest (property testing)
- **Entry point:** `src/main.rs` — creates shared state + channel, spawns MCP server on background thread + egui on main thread. Bot connection is spawned on a dedicated OS thread from the UI (azalea's `ClientBuilder::start` internally creates a `LocalSet` which is `!Send`, preventing `tokio::spawn` on a multi-threaded runtime).
- **Library crate root:** `src/lib.rs`

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Run | `cargo run` |
| Test (unit) | `cargo test --lib` |
| Test (all) | `cargo test` |
| Proptest | `cargo test --test proptest` |
| Integration | `cargo test --test integration` |
| Lint | `cargo clippy --all-targets` |
| Doc | `cargo doc --no-deps` |

Requires Rust nightly (`rust-toolchain.toml` pins nightly). Dev profile uses `opt-level = 1` (with `opt-level = 3` for dependencies) for faster iteration.

## Architecture

Layered: **types → logic → state → bot → mcp → ui**, all in one crate.

```
src/
  types.rs            — Shared data types (BlockPos, BotCommand, WorldSnapshot, etc.)
  error.rs            — BotError enum (actionable error variants for AI agents);
                        re-exports BlockPos/ToolType/MaterialTier from types.rs
  config.rs           — AppConfig (UI settings) + RunStats (atomic counters)
  state.rs            — SharedState (thread-safe hub: ArcSwap for snapshot, RwLock for config,
                        AtomicBool for online/connecting/disconnect flags)
  block_data.rs       — Block/tool/material tables, best-tool selection
  mining_calc.rs      — Mining time formulas (the single canonical calculate_mine_time)
  command_validate.rs — Coordinate validation and command pre-checks
  snapshot.rs         — World snapshot data structures and chunk summaries
  tool_select.rs      — Best-tool selection logic
  compound_ops.rs     — Compound operations (e.g. mine-and-collect pipeline)
  channel.rs          — Cross-thread BotCommand channel (tokio mpsc + oneshot);
                        ReceiverSlot + ReceiverLease for executor lifecycle
  logging.rs          — tracing-subscriber setup (stderr only; stdout = MCP transport)
  bot/                — Minecraft bot lifecycle
    mod.rs            — Re-exports
    connection.rs     — ConnectionManager (connect, reconnect with backoff, disconnect)
    events.rs         — azalea event handlers (player position, chunk loads, chat)
    commands.rs       — BotCommand → azalea action execution
    ops.rs            — Higher-level bot operations (move, mine, place, etc.)
    snapshot_updater.rs — Periodically snapshots world state into SharedState
  mcp/                — MCP server
    server.rs         — McpBotServer (rmcp ServerHandler), stdio transport
    tools_*.rs        — Tool definitions organized by domain: query, movement, block,
                        item, container, combat, chat
  ui/                 — Desktop UI
    app.rs            — egui app shell
    settings.rs       — Settings panel
    status.rs         — Status panel with live stats
tests/
  integration.rs      — Mock-based end-to-end tests (no real MC server)
  proptest.rs         — Property-based tests for block_data, command_validate
```

## Conventions

- **Doc comments (`//!`)** on every module, doc comments (`///`) on every public type and function.
- **Section separators:** `// ═══`, `// ----`, or `// ≡≡≡` lines between logical sections within a file.
- **Error handling:** `BotError` (thiserror) for all bot/MCP errors; `anyhow` for top-level main errors; `eyre` available but rarely used.
- **Thread safety:** `Arc<SharedState>` shared across threads; `ArcSwap` for lock-free snapshot reads; `RwLock` for config/stats; `AtomicBool` for online flag; `Mutex` for container handle and chat messages.
- **Testing:** Unit tests in `#[cfg(test)] mod tests { .. }` at bottom of each source file; integration tests in `tests/`; property tests with `proptest` crate.
- **Logging:** `tracing` crate macros only; all output to stderr (`stdout` reserved for MCP JSON-RPC transport). Enabled via `init_logging()` called once at startup.
- **Naming:** Types PascalCase, enums PascalCase, functions/methods snake_case, module file names snake_case.
- **MCP tools:** Each tool module (tools_*.rs) exposes a builder function; tool parameters annotated with `#[derive(schemars::JsonSchema)]`.
- **Formatting:** No `.rustfmt.toml` — uses default `rustfmt`.

## Notes

<!-- Quick-add space for future notes -->
- **规范:** 函数式编程，大量注释。写完后使用`cargo fmt`格式化；及时编写`cargo test`自动化测试，`cargo test`全过才能交付，编写遵循TDD；需要运行`cargo clippy`检验，全过才能交付；最后更新`README.md`、`CHANGELOG.md`和`AGENTS.md`，然后提交并推送git。
- **Settings panel:** Uses `EditConfig` local edit buffers (in `app.rs`); fields rendered via `TextEdit::singleline`/`DragValue`. Edits applied to `SharedState` only on Connect. The `sender` parameter was removed from `settings_panel` — the UI doesn't send commands directly.
- **Bot connection:** Spawned on dedicated OS thread (not `tokio::spawn`) because `ConnectionManager::connect()` contains `LocalSet` which is `!Send`. The thread's `JoinHandle` is held by `MinecraftApp` and joined on `Drop` for clean exit.
- **Command executor:** Wired into `Event::Spawn` via `spawn_local`. The command receiver is stored in a `ReceiverSlot` (`Arc<Mutex<Option<BotCommandReceiver>>>`) and leased out via `ReceiverLease` on Spawn; when the executor is aborted on `Event::Disconnect`, the lease drops and returns the receiver to the slot for the next reconnect. `Event::Disconnect` also calls `bot.exit()` so `ClientBuilder::start` returns and the connect loop can retry.
- **Connect/Disconnect:** `SharedState` has `bot_connecting` (AtomicBool) and `disconnect_requested` (AtomicBool) flags. `try_begin_connecting` guards against double-spawn. `request_disconnect` tells the reconnect loop to stop; the Disconnect button sets it. `clear_connecting` is called when the connect loop exits.
- **Type unification:** `error.rs` re-exports `BlockPos`, `ToolType`, `MaterialTier` from `types.rs` — no duplicate definitions. `ToolType` has 7 variants (Pickaxe, Axe, Shovel, Hoe, Sword, Shears, Hand). No `to_error_*` bridge helpers needed.
- **Snapshot building:** `handle_tick` delegates to `SnapshotUpdater::update_from_tick` — the inline `build_and_update_snapshot` and helper functions were deleted from `events.rs` to avoid duplication with `snapshot_updater.rs`.
- **Mutex poisoning recovery**: Extended from `SharedState` to all shared mutexes (including `channel.rs` command receiver slot and `bot/events.rs` executor handle). All use `.unwrap_or_else(|e| e.into_inner())` to prevent cascade crashes.
- **Command timeout:** `BotCommandSender::with_timeout` honours `AppConfig::command_timeout_secs`; `main.rs` wires the configured value at startup.