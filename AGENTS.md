# minecraft-mcp-rs — MCP server that controls a Minecraft bot

An MCP (Model Context Protocol) server backed by an actual Minecraft bot via the azalea library. Exposes bot capabilities (movement, block manipulation, inventory, combat, chat) as MCP tools consumable by LLM clients. Ships with an egui desktop UI for status and settings.

- **Stack:** Rust nightly (edition 2024; azalea 0.15.1's build script requires nightly), azalea 0.15.1 (Minecraft 1.21.11 bot), rmcp (MCP server), egui/eframe 0.34.3 (desktop UI), tokio 1.50 (async runtime), bevy_ecs 0.18 (azalea's ECS), schemars 1.0 (MCP JSON schemas), proptest (property testing)
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

Requires Rust nightly (edition 2024; `rust-toolchain.toml` pins nightly — azalea 0.15.1's build script enforces this). Dev profile uses `opt-level = 1` (with `opt-level = 3` for dependencies) for faster iteration.

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
  utils.rs            — Common utility helpers (e.g. `to_snake_case`)
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
    server.rs         — McpBotServer (rmcp ServerHandler), stdio or HTTP transport
    tools_*.rs        — Tool definitions organized by domain: query, movement, block,
                        item, container, combat, chat, plus unified `act`
    render.rs         — Top-down PNG world rendering for `get_world_view`
  ui/                 — Desktop UI
    app.rs            — egui app shell
    fonts.rs          — CJK system font auto-detection + injection into egui FontDefinitions
    i18n/             — UI internationalization (functional, one file per language)
      mod.rs          — Language enum, TextKey enum, thread-safe current()/set()/tr()
      en.rs           — English lookup table
      zh_cn.rs        — Simplified Chinese lookup table
    settings.rs       — Settings panel (includes MCP transport / port / token / language)
    status.rs         — Status panel with live stats
    mcp_config.rs     — Copyable, live-generated MCP client JSON config
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
- **MCP Config panel:** Renders a copyable JSON config (with the executable's absolute path resolved at runtime) for MCP clients like Claude Desktop / Cursor. Uses egui 0.34.3's clipboard API; schemars 1.0 `Schema` (via `schema_for!`) drives any schema rendering.
- **Bot connection:** Spawned on dedicated OS thread (not `tokio::spawn`) because `ConnectionManager::connect()` contains `LocalSet` which is `!Send`. The thread's `JoinHandle` is held by `MinecraftApp` and joined on `Drop` for clean exit. `Drop::join` has a 3-second timeout so closing the window never hangs.
- **Command executor:** Wired into `Event::Spawn` via `spawn_local`. The command receiver is stored in a `ReceiverSlot` (`Arc<Mutex<Option<BotCommandReceiver>>>`) and leased out via `ReceiverLease` on Spawn; when the executor is aborted on `Event::Disconnect`, the lease drops and returns the receiver to the slot for the next reconnect. `Event::Disconnect` also writes `AppExit::Success` to the ECS (`bot.ecs.lock().write_message(AppExit::Success)`) so `ClientBuilder::start` returns and the connect loop can retry (azalea 0.15.1 removed `Client::exit()`).
- **Connect/Disconnect:** `SharedState` has `bot_connecting` (AtomicBool) and `disconnect_requested` (AtomicBool) flags. `try_begin_connecting` guards against double-spawn. `request_disconnect` tells the reconnect loop to stop; the Disconnect button sets it. `clear_connecting` is called when the connect loop exits. A `CancellationToken` (from `tokio-util`) is stored in `SharedState` and cancelled on disconnect so the reconnect backoff sleep returns immediately instead of blocking shutdown.
- **Connection errors:** `SharedState::last_error` (behind a `Mutex<Option<String>>`) surfaces the most recent connection failure to the UI; the Status panel renders it in red. Connection failures are fail-fast — the reconnect loop stops retrying so the user sees the error and can manually retry, rather than looping infinitely.
- **Type unification:** `error.rs` re-exports `BlockPos`, `ToolType`, `MaterialTier` from `types.rs` — no duplicate definitions. `ToolType` has 7 variants (Pickaxe, Axe, Shovel, Hoe, Sword, Shears, Hand). No `to_error_*` bridge helpers needed.
- **Snapshot building:** `handle_tick` delegates to `SnapshotUpdater::update_from_tick` — the inline `build_and_update_snapshot` and helper functions were deleted from `events.rs` to avoid duplication with `snapshot_updater.rs`.
- **Mutex poisoning recovery**: Extended from `SharedState` to all shared mutexes (including `channel.rs` command receiver slot, `bot/events.rs` executor handle, and `logging.rs` test helpers). All use `.unwrap_or_else(|e| e.into_inner())` to prevent cascade crashes.
- **`SharedState::modify_snapshot` 约定:** 事件 handler 更新快照部分字段（entities / self_player.health 等）**必须**用 `SharedState::modify_snapshot<F: FnMut(&mut WorldSnapshot)>(&self, f: F)`，基于 `ArcSwap::rcu` 原子读-改-写。闭包签名为 `FnMut(&mut WorldSnapshot)`（**不是 `FnOnce`**，因为 `rcu` 在并发更新下可能重试）。**禁止**再用 `read_snapshot().clone()` + `update_snapshot()` 模式 —— 该模式会丢更新（`SnapshotUpdater` 在 await 点交错时会把字段原样写回，抹掉事件 handler 的修改）。`handle_death` / `add_player_to_snapshot` / `handle_remove_player` / `handle_update_player` 已迁移到此 API。
- **`command_sender` 注入链路约定:** `Act::Mine` 需要委托 `CompoundOpExecutor` 完整执行（选工具→走过去→挖掘→验证），而 `CompoundOpExecutor` 需要 `BotCommandSender` 才能发 `BotCommand`。由于 azalea ECS 通过 `BotState::default()` 构造 `BotState`，无法直接传参，采用与 `INJECTED_SHARED_STATE` / `INJECTED_COMMAND_RECEIVER` 同模式的 `OnceLock` 注入：`ConnectionManager::connect(command_receiver, egui_ctx, command_sender)` 在入口 `events::INJECTED_COMMAND_SENDER.set(Some(command_sender))`；`BotState::default` 读取该 static 得到 `command_sender: Option<BotCommandSender>`；`handle_spawn` 在 `async move` 闭包**外**提前 `let command_sender = state.command_sender.clone();`（避免 E0521 borrowed data escapes）传入 `CommandExecutor::new_for_lease(client, shared_state, command_sender)`；`CommandExecutor::sender: Option<BotCommandSender>`；`handle_act(Mine)` 当 `self.sender.is_some()` 时构造 `CompoundOpExecutor::new(sender.clone(), Arc::clone(&self.state))` 并调用 `execute_mine_block(pos, true)`，sender 为 None 时（单元测试 mock 执行器）回退到 `handle_break_block` 并 `warn!` 日志。`MinecraftApp` 持有 `sender: BotCommandSender` 字段（来自 `main.rs` 的 `sender_for_egui`），`connect_bot` clone 一份传入 `manager.connect(...)`。
- **`shutdown_token` 生命周期约定:** `SharedState` 持有 `shutdown_token: Mutex<CancellationToken>`（来自 `tokio-util`，与 `cancel_token` 字段独立 —— `cancel_token` 用于打断 reconnect backoff sleep，`shutdown_token` 用于 MCP graceful shutdown，两者生命周期独立）。通过 `shutdown_token() -> CancellationToken` 暴露（克隆返回）、`trigger_shutdown()` 触发。`MinecraftApp::drop` 调用 `trigger_shutdown()`；`serve_http` 在 `app` 构造前 `let shutdown_token = state.shutdown_token();`（因为 `.with_state(state)` 消费 state），再用 `axum::serve(listener, app).with_graceful_shutdown(async move { shutdown_token.cancelled().await; })`（**必须用 `async move` 包装** —— `CancellationToken::cancelled()` 返回的 future 借用 token，非 `'static`，不满足 `with_graceful_shutdown` 的 `Send + 'static` 约束）；`serve_stdio` 在 `McpBotServer::new(state, sender)` 前 `let shutdown_token = state.shutdown_token();`，再用 `tokio::select!` 并联 `running.waiting()` 与 `shutdown_token.cancelled()`，shutdown 触发时立即返回不等 stdin EOF。MCP 线程 `JoinHandle` 现存于 `MinecraftApp`，drop 时 join（3 秒超时，与 bot 线程一致）。
- **`tick_abort_handles` 约定:** `BotState` 新增字段 `tick_abort_handles: Arc<Mutex<Vec<AbortHandle>>>`，追踪 `handle_tick` 中 `spawn_local` 的 tick 任务 `AbortHandle`。`handle_disconnect` 时 abort 全部并清空，防止 ECS 拆除后 tick 任务调用 `bot.component()` / `bot.world()` 触发 panic 或写入陈旧快照。模式与 `executor_handle` 一致（`take()` + abort）。
- **Command timeout:** `BotCommandSender::with_timeout` honours `AppConfig::command_timeout_secs`; `main.rs` wires the configured value at startup.
- **UI i18n:** Functional, one-file-per-language module under `src/ui/i18n/`. `Language` enum (`En`, `ZhCn`) with `Default = En`; `TextKey` enum enumerates all UI strings; `tr(key) -> &'static str` is a pure dispatch to the current language's `lookup`. Current language is held in a `static RwLock<Language>` (`current()`/`set()`). `AppConfig::language` persists the choice (`#[serde(default)]` for backward compat). The Settings panel Language dropdown calls `i18n::set()` on change for next-frame effect (no reconnect). MCP tool descriptions and JSON field names stay English (external API contract).
- **CJK fonts:** `src/ui/fonts.rs::install_system_cjk_fonts(ctx)` probes the platform default CJK font (Windows `msyh.ttc`, macOS `PingFang.ttc`, Linux Noto/WenQuanYi) at startup and injects it into egui `FontDefinitions` (prepended to `Proportional`, appended to `Monospace`). Falls back to the default font with a `warn` log if none found — never panics. Called once in the eframe creation closure.
- **CI (build):** `.github/workflows/build.yml` matrix-builds the release binary for Linux × x86_64/aarch64, macOS aarch64 only, and Windows × x86_64/aarch64 using native ARM runners (`ubuntu-24.04-arm`, `macos-latest`, `windows-11-arm`). Artifacts named `minecraft-mcp-rs-<os>-<arch>`.
- **Docs (VitePress):** `docs/` holds a bilingual (English + 简体中文) VitePress site. Config split mirrors vuejs/vitepress: `config/index.ts` assembles `locales` (root=EN, `zh`=ZH); `en.ts`/`zh.ts` hold per-locale nav/sidebar/theme text. `base` reads `BASE_PATH` env var (default `/minecraft-mcp-rs/`) for GitHub Pages project sites. `.github/workflows/deploy-docs.yml` builds with Node 24 and deploys via `actions/deploy-pages@v5`. Run locally: `npm install && npm run docs:dev`.
- **Dependency patches:** `patches/rmcp/` and `patches/rsa/` are tracked in git and required by the `[patch.crates-io]` section in `Cargo.toml`. Do not add them to `.gitignore`; CI and fresh clones need them on disk. When updating either patch, keep the upstream license files intact and document the change in `README.md` / `CHANGELOG.md`.
- **GitHub Actions runtime:** workflows under `.github/workflows/` use the latest Node.js LTS/runtime supported by GitHub-hosted runners. When GitHub deprecates an old Node runtime, upgrade official actions to their newest major version and bump `node-version` accordingly; document the change in `CHANGELOG.md`.
- **Runner maintenance:** when a GitHub-hosted runner image is deprecated, suffers from long queues, or is otherwise unmaintainable, drop the affected target or migrate it to the latest stable `-latest` label, then document the change in `CHANGELOG.md`.
- **Injection globals are `Mutex<Option<T>>`:** the four values injected into `BotState::default` (`INJECTED_SHARED_STATE`, `INJECTED_COMMAND_RECEIVER`, `INJECTED_EGUI_CTX`, `INJECTED_COMMAND_SENDER`) live in `Mutex<Option<_>>` rather than `OnceLock` so they can be reset on disconnect / between tests. `ConnectionManager::connect` sets them before entering the azalea loop; `handle_disconnect` clears them all back to `None`. This keeps reconnects and unit tests from seeing stale injection state.
- **`BotCommand::UseItemWithSlot`:** use this single command when an MCP tool needs to "switch to slot X and then use the item". The switch and the use happen inside the same executor critical section, so concurrent commands cannot interleave a different `SwitchHotbarSlot` between them. `tools_item::handle_use_item` sends `UseItemWithSlot(slot)` when `item_slot` is provided, otherwise falls back to parameterless `UseItem`.
- **Nearby queries are parameterised:** `get_nearby_blocks` and `get_nearby_entities` accept a `radius` argument (and `get_nearby_blocks` also accepts an optional `filter_type` substring). The tool input structs derive `Deserialize` and `rmcp::schemars::JsonSchema` and are wired through `Parameters<T>` in `server.rs`; the handlers no longer hardcode a radius.
- **MCP tool schemas use derive:** input structs in `tools_*.rs` use `#[derive(Deserialize, rmcp::schemars::JsonSchema)]`. Hand-written `impl JsonSchema` is reserved for cases where the derive truly cannot express the schema; adding it just to work around an old `schemars`/`rmcp` conflict is not acceptable because it drifts silently when fields change.
- **Tool schema range annotations:** bounded numeric MCP parameters (e.g. `WalkDirectionInput.distance`, `SwitchHotbarSlotInput.slot`) carry `#[schemars(range(min = ..., max = ...))]` so clients see the limits in the generated JSON Schema.
- **Common `to_snake_case` helper:** `src/utils.rs::to_snake_case` is the single implementation for converting azalea registry variant names (e.g. `IronPickaxe`) into snake_case item ids. Both the command executor and snapshot updater import it from `crate::utils`; do not re-implement it locally.
- **Deprecated `block_data::find_best_tool_in_inventory`:** this function is `#[deprecated(note = "use tool_select::find_tool_in_inventory instead")]` and delegates to `tool_select::find_tool_in_inventory`. New code and tests should call the `tool_select` version directly; the deprecated re-export remains only for backward compatibility.