# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`place_block` / `use_item_on_block` verification no longer races the
  idle snapshot relaxation.** The auto-approach leg dispatches nothing, so
  a walk longer than the 3 s activity window let the snapshot cadence
  relax to 5 s while the verification budget stayed at
  `snapshot_interval_ms + 250` ms — an accepted placement was reported as
  "did not appear ... likely rejected". Both handlers now re-stamp command
  activity when the cadence is idle (the fix `execute_mine_block` got in
  1.4.2).
- **`give_item` propagates the executor's result through the
  `/item replace` step.** The arm bound the result as `Ok(_)` and
  hardcoded `success: true`, answering "Gave Nx ... into hotbar slot N"
  even when the executor reported `success: false` (server accepted the
  command but did not honour it).
- **The headless supervisor's quiet-wait no longer consumes the
  config-restart flag when a bot thread has appeared.** `connect_bot`
  spawning a thread while the supervisor waited went unnoticed (the wait
  polled only shutdown + the flag), so an online `update_settings` restart
  was consumed and silently lost. The extracted `quiet_wait_step`
  re-checks `SharedState::bot_thread_running` before consuming.
- **`smart_move`'s retry is bounded by the command envelope.** A retry
  after a Completed-but-unreached first attempt ran a full fresh goto
  window, so first attempts failing after >0.7 s overran
  `command_timeout` and the structured `{reached, obstacle, retried}`
  result was dropped for a bare `CommandTimeout`. The retry now gets only
  the remaining window and is skipped when <2 s is left.
- **The executor's container fallbacks return `ContainerNotOpen`
  (-32010)** instead of generic `Internal` (-32603) — the dedicated
  runtime-state variant clients were told to branch on. `execute_open_
  container` keeps the executor's real failure message instead of
  replacing every non-success with `ContainerTimeout`, and
  `execute_equip_tool` feeds dispatch errors through the state machine
  like every other arm.
- **Stale cross-session commands are drained at executor start.**
  Commands buffered while no executor held the receiver (offline window /
  aborted previous session) used to execute in the NEXT session with all
  responses lost. Each is now answered immediately with an honest
  `Offline("stale command from a previous session; reconnect and retry")`.
- **`snapshot_interval_ms` takes effect immediately.** It was frozen into
  `BotState` at connect time, so `update_settings` reported `applied` but
  the running session kept the old cadence until reconnect. The tick now
  reads it live from the config; the injection static is removed.
- **`break_block` distinguishes a gone block from an unobserved one.** An
  absent snapshot entry inside the retention bound
  (`max(chunk_scan_radius, 8)` chunks) reports `BlockNotFound` (the block
  was mined); outside it, `ChunkNotLoaded` remains.
- **`drop_item`'s container-window branch reports `success: false`.** The
  stack may have been shift-clicked into the container instead of thrown;
  claiming "Dropped N item(s)" contradicted `verified: false`.
- **Harvest levels corrected to the vanilla 1.21 tags.** `lapis_block`
  0→1 (needs_stone_tool) and `redstone_block` 0→2 (needs_iron_tool) — the
  old entries let a wood pickaxe break them for no drops;
  `netherite_block` 4→3 (needs_netherite_tool is empty, needs_diamond_tool
  applies) — a diamond pickaxe is no longer refused a legal mining.
- **`crying_obsidian` / `respawn_anchor` added to all mining tables**
  (hardness 50, diamond-tier, drop-gated). They previously fell through to
  Hand + hardness 1.0: a 1.5 s budget against a real 250 s hand-break with
  no tool refusal.
- **`stone_slab` hardness 1.5 → 2.0** (vanilla; every sibling slab was
  already 2.0).
- **The headless HTTP bind resolves `localhost` by name.** The only
  consumer of `mcp_address` coerced every non-IP spelling to 127.0.0.1;
  `resolve_bind_addr` now passes IP literals through, resolves hostnames
  via the OS resolver, and keeps the warn + loopback fallback.
- **The Settings panel's `apply` merges only dirty fields under the
  config write lock**, so a concurrent `update_settings` landing inside
  the old read-modify-write window is no longer silently reverted.
- **The MCP Config panel shows a pending-edits hint** while any edit
  buffer is dirty (the copyable JSON can differ from the running config
  until Connect), and its TLS warning suppresses the empty-address case
  like the Settings panel.
- **Doc drift:** `get_bot_status`'s description says `snapshot_timestamp`
  (epoch ms), not "snapshot age"; the `Language` doc no longer claims
  persistence across restarts; the `last_language` cache doc matches what
  it actually skips; the mining-calc ice test comment no longer claims
  ice is absent from `BLOCK_TO_TOOL_TYPE`.

### Changed

- **`get_world_view`'s cache key check probes
  `SharedState::world_view_cache_meta()` first**, avoiding a ~700 KB PNG
  clone on every cache miss. No wire change.
- **Honest test rewrites:** `prop_higher_tier_faster_or_equal` compared
  tiers in strictly ascending speed order, so its assertion body never
  executed (a complete tautology); it now compares all unordered
  same-side-of-the-harvest-gate pairs and genuinely asserts. The gold
  round-trip oracle expects the `golden_` prefix the parser accepts.
  `test_subsecond_timeout_not_truncated` exercises a real 200 ms
  `send_command_with_timeout` envelope; the receiver-lease retry test
  drops the real lease instead of planting a foreign receiver;
  `test_auto_reconnect_sequence_simulation` renamed to
  `test_online_offline_snapshot_reseed_flow` (it never exercised a
  reconnect loop); the offline-connect test binds and drops an ephemeral
  port instead of hardcoding port 1.

## [1.4.2] - 2026-08-28

### Fixed

- **Mining verification no longer races the idle snapshot relaxation.**
  `execute_mine_block` computed its Step-10 verification budget from the
  *configured* snapshot interval, but the updater relaxes to ≥5 s once no
  command has been dispatched for 3 s — and a slow-but-legal mine (hand-
  mining oak_log is exactly 3.0 s) sleeps through that whole window without
  dispatching anything. Successful breaks were reported as
  `MiningInterrupted("block still present after mining time")`. The state
  machine now re-stamps command activity after the mine sleep (decision via
  the new `SharedState::snapshot_cadence_idle` / shared
  `SharedState::within_activity_window` helpers), putting the updater back
  on the fast cadence before verification polls.
- **The open-container handle is torn down on disconnect.**
  `clear_session_state` missed `container_handle`, so a chest left open at
  disconnect survived into the next session: `open_container` failed with
  `ContainerAlreadyOpen` and `take/put_from_container` shift-clicked the
  dead session's menu while reporting success.
- **`give_item` reports an honest message when the swap-click fallback
  fails.** The `/item replace`-rejected fallback used to answer
  "moved into hotbar slot N via swap-click" even when `success:false`, and
  discarded the executor's failure reason; it now says
  "swap-click hotbar move failed: <reason>".
- **The Settings and MCP Config panels track agent-driven config changes.**
  The UI edit buffer was initialised once and never refreshed, so
  `update_settings` tool calls were invisible to both panels (the MCP Config
  panel could hand out a stale token). Untouched fields now re-sync from the
  live config every frame (`EditConfig::sync_untouched_from`); locally dirty
  fields keep their in-progress values until the user clicks Connect.
- **The Creative-mode hint is part of the registered tool descriptions.**
  The hint text existed only in dead `tools_block.rs` constants asserted by
  a self-referential test; clients never saw it. `break_block` /
  `place_block` descriptions now carry it, asserted against the live tool
  registry in `server.rs::tests`.
- **`get_nearby_blocks(top_only)` drops air entries itself** instead of
  relying solely on the updater's air-entry invariant, using the same
  predicate as the renderer (`air`/`cave_air`/`void_air`).
- **OS Ctrl+C handlers are registered in headless mode only** (2026-08-26
  review round). Both `serve_stdio` and the HTTP path's `shutdown_signal`
  raced a Ctrl+C signal unconditionally; registering the handler replaces
  the OS default process-wide, so in UI mode a terminal Ctrl+C silently
  stopped just the MCP transport while the egui window lived on as a
  zombie. The ctrl_c arms are now gated behind `headless` like the stdio
  idle watchdog (`serve_http`/`shutdown_signal` take the run mode through);
  in UI mode Ctrl+C falls back to OS default — terminating the process.
- **Forced snapshot refresh resolves only after a successful build.**
  The build task used to signal the force-request oneshot regardless of
  build success, releasing `force=true` callers early with a pre-refresh
  snapshot. On failure the sender is now returned to the single slot
  (only while empty — newer requests win) so the updater's 250 ms
  failure-retry rebuild completes it; the caller's own 3 s timeout stays
  the backstop.
- **Non-finite yaw readings no longer reach annotations.** A NaN/±∞ look
  angle would propagate through `normalize_yaw` verbatim into
  `SelfPlayer::yaw`; the single write point now uses
  `normalize_yaw_checked`, folding non-finite input to `None`
  ("yaw unknown" — the renderer skips the heading arrow).
- **TLS warnings agree with validation on loopback.** The Settings and
  MCP Config panels hard-coded `"127.0.0.1" | "::1" | "localhost"` as the
  safe list while `validate()` accepts the whole loopback range, so a bind
  to e.g. `127.0.0.2` validated fine but was shown in red. Both panels now
  call config.rs's `is_loopback_bind_address`.

### Changed

- **Position-bound rejection wording unified.** `validate_position` now
  delegates to `validate_coordinates` (single source of truth for the world
  border / build-height bounds), so dispatch-gate rejects read
  "x coordinate ... out of range (...)" instead of "X coordinate ... out of
  bounds (...)". The CollectItems radius check is likewise shared by the
  `BotCommand::CollectItems` arm and `validate_act_action` (message
  unchanged). Wire-visible only in error message casing.
- **`equip_tool` routes through `send_and_serialize`**, matching the
  "single sanctioned form" contract every other uncompensated tool follows.
- **Documentation corrections:** the HARVEST_LEVEL header documents the
  `u8::MAX` bedrock sentinel (no tool satisfies it; alternatives
  intentionally empty), and the npx/bunx launcher JSON comments describe
  their LazyLock once-per-process build instead of calling them
  compile-time constants.

### Tests

- **Exhaustive envelope-class guard for `timeout_for`.** The envelope
  classifier uses `matches!` rather than a match, so a new `BotCommand` /
  `ActAction` variant would silently take the plain command envelope. A
  test-side exhaustive match over all 28 variants (nested over all six
  `ActAction`s) now makes that a compile error and asserts the actual
  timeout class per variant.

## [1.4.1] - 2026-08-24

### Fixed

- **CI: the release-branch publish gate moved into the `mode` job.** The
  guard refusing a suffix-less version on a `release`-branch push used to
  live inside the `release` job, which `npm-publish` does not `need` —
  pushing the finalized `1.4.0` to `release` tripped the guard yet still
  published all six packages under npm `next`, burning the stable name
  before the tag run and leaving `latest` stale. The `mode` job now checks
  out the tree and rejects such pushes up front (~20 s instead of a full
  matrix); `build` needs `mode` so nothing builds either, and the
  in-`release` guard remains as defense-in-depth.
- **CI: the already-published dist-tag fallback is auth-aware.** PR #38's
  `npm dist-tag add` re-tag cannot be authorized by Trusted-Publishing
  OIDC (which covers `npm publish` only), so the first already-published
  package hard-failed the whole loop with E401. Without an `NPM_TOKEN`
  secret the fallback now emits a `::warning::` carrying the exact local
  remediation (`npm dist-tag add <pkg>@<ver> <latest|next>`) and keeps
  going; with a token configured it still hard-fails on genuine errors.
- **CI: `HOMEBREW_NO_REQUIRE_TAP_TRUST=1` set in both workflows.** GitHub's
  macOS runner images ship third-party taps (`aws/tap`) that newer Homebrew
  flags as untrusted; image maintenance scripts surface trust errors in job
  logs that read like build failures (macos-aarch64, 2026-08-24 — the build
  itself was green in every run).

### Changed

- **Docs home "quick setup" reduced to a plain JSON config block.** The
  `McpQuickSetup.vue` component (Cursor deeplink button, Claude Code / VS Code
  copy-command buttons, collapsible manual-config block) was removed together
  with its `theme/index.ts` registration. Both home pages now render a static
  `## 一键接入主流 Agent` / `## One-click setup for mainstream AI agents`
  heading followed by a single json code fence holding the same
  `minecraft-mcp-rs@latest --headless --stdio` server config; VitePress's
  built-in code-block copy button covers copying.

## [1.4.0] - 2026-08-24

### Fixed

- **CI: `setup-rust-toolchain` input renamed `targets` → `target`.** The
  action's current v1 input list no longer accepts the plural `targets`;
  every matrix job (build.yml) and release job (release.yml) logged
  `Unexpected input(s) 'targets'` annotations and silently skipped the
  explicit `rustup target add`. Native-arch runners masked the functional
  impact (host triple == build target), but any future cross-compilation
  target would have failed. Both workflows now pass the singular `target`
  input.

### Added

- **Application icon (window + Windows executable).** The Minecraft-themed
  block/gear artwork now ships as `assets/icon.png` (512×512 RGBA) and
  `assets/icon.ico`. The egui window icon is set at startup via the new
  `ui::icon::load_app_icon()` (`include_bytes!` + `eframe::icon_data::
  from_png_bytes`, warning + platform-default fallback on decode failure).
  A new `build.rs` embeds the `.ico` plus version info into the PE resource
  section via `winresource` (build-dependency), gated on
  `CARGO_CFG_TARGET_OS == "windows"` so non-Windows builds are unaffected;
  `[package]` gained the `description` / `license = "MIT"` metadata the
  version-info sheet displays. `build.yml` path filters now include
  `assets/**` and `build.rs`.
- **Documentation-site branding:** `docs/public/logo.png` serves as the
  favicon (wired through the new `head` entry in `config/index.ts`), the
  navbar logo (`themeConfig.logo` in both `en.ts` and `zh.ts`), and the home
  hero image (EN + ZH). Note: VitePress does **not** auto-prefix `head`
  URLs with `base`, so the favicon href concatenates the configured base
  explicitly; `themeConfig.logo` and hero images go through `withBase()`
  and stay root-absolute.

### Changed

- **README header:** centered project logo (120 px, from
  `assets/icon.png`) above the status/nightly badges.

## [1.3.2] - 2026-08-23

### Fixed

- **`update_settings` tool description no longer claims "Persisted to the
  config file"** — config files were removed (S-8); settings are env-var
  sourced and in-memory at runtime. The old wording misled MCP clients into
  believing runtime changes survive a restart.
- **Unified offline error wording across the tool surface:** the 23
  hand-rolled `is_online()` gates had drifted to three different messages.
  They now all route through the new `mcp::common::require_online`, whose
  single message is `"Bot is currently offline"`. The repeated
  send-command-and-serialize tail collapsed into
  `mcp::common::send_and_serialize`.
- **`tests/integration.rs` variant guard updated to 28:** the runtime list
  was silently missing `MoveItemToHotbar` (added 1.1.5) and asserted 27.
- **Doc drift:** `SelfPlayer::yaw` docs now consistently say DEGREES (one
  site claimed radians); `AppConfig::mcp_token` doc dropped the stale
  "persists across restarts"; AGENTS.md's `nether_gold_ore` entry corrected
  to the M-21 level-0 adjudication; AGENTS.md's `act` result entry corrected
  to the L-15 SelfPlayer-clone override.

### Changed

- **npm launcher hardening for npx/bunx:** the launcher spawns with
  `windowsHide: true` and self-heals a lost executable bit on POSIX
  (`EACCES`/`ENOEXEC` → one `chmod 0o755` + retry), so cached npx/bunx runs
  no longer dead-end on mode-stripped tarball extraction.

### Docs

- **New "Building from Source" tutorial** under `/dev/building` (bilingual),
  covering prerequisites, the pinned-nightly rationale, the full test/lint
  gate suite, release artifacts, and common build issues; build details
  moved out of Getting Started, which now leads with the zero-Rust
  `npx`/`bunx` quick run.
- **Install flows lead with `npx`/`bunx`** across README, the npm page, and
  Getting Started; global install demoted to secondary. The npm page's
  stale "config file / `--config`" section was replaced by a
  `MINECRAFT_MCP_*` quick-reference table (the mechanism was removed in S-8).
- **Tool count stated precisely as 41** everywhere ("30+" remnants removed).
- **VitePress theme polish:** industrial-block styling (Chakra Petch /
  IBM Plex faces, blueprint-grid hero, pixel-block accents, staggered load
  reveal). Colors are untouched — every rule consumes VitePress default CSS
  variables only.

### Added

- **`BotError::EntityNotFound` / `UNKNOWN_ENTITY_ID`:** entity-not-found now
  travels as `RESOURCE_NOT_FOUND` with `reason: entity_not_found` + `entity_id`
  (F-34), and entities missing from the live ECS index use the `u32::MAX`
  sentinel instead of collapsing to id 0 (F-27).
- **Real JSON-RPC dispatch-layer tests:** the generated 41-tool registry is
  snapshotted (names + read/destructive annotations), `tools/call` is driven
  through an in-memory rmcp duplex transport (happy path, unknown tool, and
  malformed arguments), and the axum auth wrapper's 401 shape is exercised
  with `tower::ServiceExt::oneshot` (F-3). `tower` is now a dev-dependency.

- **`BotActions::goto_with_deadline`:** the fly timeout now reaches the
  pathfinder itself (audit M-1). `goto_with_margin_with_timeout` passes its
  deadline through to the new `goto_with_deadline` (default: plain `goto`),
  so a flight with `fly_timeout_secs > command_timeout_secs` no longer dies
  at the 30 s command envelope with `PathfindingFailed("pathfinding timed
  out after 30s")`.
- **`BotCommandSender::send_command_with_timeout`:** explicit per-call
  timeout override on the command channel, used to bound the live `/seed`
  commands probe to 3 s (audit L-18).
- **`SharedState::world_view_cache_meta()` + `WorldViewCacheMeta`:** the UI
  preview panel now reads only the lightweight annotation + `snapshot_seq`
  pair instead of cloning the whole cache entry (up to ~700 KB of base64 PNG
  per frame) just to decide whether to rebuild its texture (audit M-11).
- **UI `McpConfigCache` + `ChatCache`:** per-frame allocation cuts (audit
  L-19/L-20). The MCP Config panel caches its pretty-printed JSON and the
  resolved executable path, rebuilding only when the config changes; the
  Status panel caches the chat-log rendering until the chat cursor moves.
- **Single-entry response cache for `get_nearby_blocks`:** keyed on
  `(snapshot_seq, radius, filter_type, top_only, max_blocks)` so an LLM
  polling loop skips the O(n) full-snapshot scan on every call (audit L-17).
- **`BotError::ContainerNotOpen`:** "no container is currently open" now
  maps to its own JSON-RPC code -32010 (`reason: container_not_open`,
  retryable false) so MCP clients can distinguish a runtime-state error
  from invalid parameters (audit L-9).
- **`max_entities` parameter for `get_nearby_entities`:** caps the response
  (default 500, max 10000) with an honest `truncated` flag (audit L-13).
- **Compound mining equips tools from the main inventory:** when the best
  tool lives only in the main inventory (slots 9-35), the mine flow now
  dispatches `MoveItemToHotbar` + `SwitchHotbarSlot` before digging
  (`ToolSelection` gained `item_id`) instead of silently digging with the
  held item (audit H-1).
- **Monotonic activity stamps:** `last_command_at` / MCP-request activity
  now store nanos since a process-start `Instant` anchor instead of
  wall-clock epoch milliseconds, so an NTP jump can no longer stall the
  snapshot-relaxation probe or wedge the headless idle watchdog (audit
  L-23).

### Changed

- **`send_chat` rejects `/`-prefixed messages** (F-2): azalea forwards a
  leading `/` as a server command packet, which would bypass
  `execute_command`'s rejection-feedback verification. The MCP layer now
  returns `InvalidParams` and points callers at `execute_command`.
- **Compound `Act` envelopes and budgets (F-1):** `Act(Mine)` /
  `Act(CollectItems)` use the longer `max(command, fly)` envelope like
  `Act(Fly)`; `execute_mine_block_with_budget` returns an honest partial
  result before dispatching `BreakBlock` when the mine sleep + verification
  cannot fit, and `collect_items` never rounds a per-target share above the
  remaining budget (the 2 s floor now stops the loop instead).
- **Mining-time model corrected (F-7/F-20):** wrong-tool / under-tier
  breaking now uses vanilla's independent 100-tick branch
  (`hardness × 5 / speed`, e.g. stone by hand = 7.5 s, not 11.25 s), and
  `calculate_mine_time` prices a correct-but-too-weak tool (wood pickaxe on
  iron ore) as non-harvest breaking.
- **`modify_snapshot` rebuilds `block_index`** after the mutation closure, so
  callers that mutate `blocks` no longer inherit a stale index (F-6).
- **Config validation hardened:** HTTP + non-loopback bind + auth disabled is
  rejected (F-9); `mc_address` must be a real IP/hostname (F-23);
  `from_env` reconciles `reconnect_max_delay_ms` up to
  `reconnect_initial_delay_ms` so one bad cross-field pair cannot discard the
  whole env config (F-8); the post-validation fallback re-applies `--stdio`
  (F-8).
- **Snapshot scanner uses the dimension's real `min_y`** (F-11), deferred
  overflow chunks keep their individual dirty-block entries until the full
  scan runs (F-26), and failed snapshot builds retry after 250 ms instead of
  a full interval (F-31).
- **`place_block` success message:** a verified placement whose snapshot
  inventory has not caught up reports the hotbar slot instead of the
  misleading "(empty slot)" label (F-33).
- **`walk_direction` computes its origin from the live position** with the
  snapshot as fallback (F-25).
- **Command chat/command lines are capped at 256 characters** so an LLM
  cannot trigger the vanilla "Chat message too long" disconnect (F-22).
- **Timeout documentation:** the command channel now explicitly documents
  that a timeout is not cancellation and retries can duplicate side effects
  (F-10).
- **`place_block` now places the block AT exactly `(x, y, z)`** (breaking
  wire change, audit H-2): the executor right-clicks the cell below the
  target (azalea's fixed Up-face convention), pre-checks `y` in `-63..=320`
  (`y=-64` is rejected), that the click target is loaded, and that the
  effect cell is empty, auto-approaches when farther than 4.5 blocks, and
  verifies the placement by polling the snapshot. Failures return an honest
  `success:false` instead of the historical "success" at `(x, y+1, z)`.
- **Chat tools return the serialized `BotResult` JSON object** (breaking
  wire change, audit L-12): `send_chat`, `execute_command` and
  `set_game_mode` now return `{success, message, data}` like every other
  action tool, instead of bare message strings.
- **`get_nearby_entities` returns an object** (breaking wire change, audit
  L-13): `{"entities": [...], "count": N, "truncated": bool}` instead of a
  bare JSON array, matching `get_nearby_blocks`' object shape.
- **Rustdoc link hygiene:** all 27 `cargo doc` warnings fixed — unresolved
  intra-doc links resolved or demoted to plain code spans, public docs no
  longer link private items, one redundant explicit link target removed;
  a new `[lints.rustdoc]` baseline denies `broken_intra_doc_links`,
  `private_intra_doc_links` and `redundant_explicit_links` so
  `cargo doc --no-deps` stays warning-free.

### Fixed

- **Cancel-window config restarts consume `session_was_online`** (F-12): a
  leaked latch previously made the next first-connect failure enter the
  infinite-backoff branch while clearing `last_error` (silent permanent
  reconnect).
- **`test_goto_notify_clone_shares_state` now actually polls the waiter**
  (F-15); proptest properties that were tautological or had an INFINITY
  escape hatch were strengthened (F-16); the duplicated inline throttle
  tests in `events.rs` were removed in favour of the real
  `SnapshotUpdater` coverage (F-17); process-wide i18n mutations are
  serialised by `I18N_TEST_LOCK` (F-18).
- **UI settings coverage:** a parameterized case for every `EditConfig`
  field drives dirty → apply → `read_config`, so a field whose widget forgets
  the dirty flag is caught (F-5).
- **Docs/comment drift:** rmcp version and the 41-tool count in
  `server.rs`, the truncated `cli.rs` doc sentence, the duplicated
  `plan_dirty_chunk_scan` docs, and the bogus `SeqCst` atomic-toggle comment
  were fixed; `src/mcp/mod.rs` (dead file shadowed by the inline module) was
  removed; `ConnectionManager::disconnect` was renamed to
  `simulate_offline_for_tests` and restricted to tests (F-24/F-28/F-29/F-35/
  F-37).
- **Compound mine flow no longer fails when the best tool is only in the
  main inventory (H-1):** the mining wait was computed with the best tool's
  speed while the bot actually dug with its hand-held item, so
  `wait_for_block_gone` always timed out and every `act(Mine)` /
  `break_block` (default path) returned `MiningInterrupted`. The flow now
  moves the tool to the hotbar first (see Added); the mock `mine_block`
  also enforces the held-item speed so the bug is caught by tests.
- **`place_block` no longer reports fake success or wrong coordinates
  (H-2):** see the breaking change above; a placed block now occupies the
  requested cell and the result reflects verification.
- **`nether_gold_ore` harvest level corrected to iron+ (H-3):** vanilla
  1.21 requires an iron pickaxe or better for drops; it is now level 2 in
  `HARVEST_LEVEL` instead of level 1, so a stone pickaxe is no longer
  selected for it (which previously mined with no drops while reporting
  success). The documented conservative over-requirements (`coal_ore`,
  `netherrack`, `end_stone`, `purpur`, `deepslate`, `nether_quartz_ore` at
  level 1 vs vanilla 0) are kept deliberately.
- **`fly_timeout_secs` is actually honoured (M-1):** the pathfinder no
  longer caps long flights at the 30 s command timeout.
- **Reconnect backoff counters reset after a successful session (M-2):**
  `attempt` / `first_connect_attempts` no longer grow monotonically across
  sessions; one success restores the full 3-attempt first-connect window
  and fresh backoff.
- **Config-restart during the TCP-connect window no longer drops the
  restart (M-3):** the connect-cancel branch now checks the restart flag,
  so an agent changing `mc_address` while the TCP connect is in flight
  still triggers the reconnect instead of leaving the GUI bot offline.
- **`smart_move` uses the live position for its reached check (M-4):** a
  long successful move is no longer misreported as an "obstacle" because
  the throttled snapshot lagged the arrival.
- **`walk_direction` / `collect_items` use the movement reply margin
  (M-5):** both route through the margin wrapper so the executor replies
  before the command envelope times out; `collect_items` splits
  `command_timeout - MOVEMENT_REPLY_MARGIN` across targets (2 s floor per
  target) and reports honest partial results on budget exhaustion.
- **`use_item_on_block` occupancy pre-check applies only to placement items
  (M-6):** flint-and-steel and other non-placement items no longer get
  rejected because the effect cell is occupied, nor do they trigger the
  auto-approach walk.
- **`place_block` MCP message rewrite gated on success (M-7):** a failed
  placement no longer gets its failure reason overwritten by a "Placed X
  at ..." message.
- **UI settings no longer roll back agent-applied config on Connect (M-8):**
  the edit buffer is refreshed from the current config, so
  `update_settings` changes survive a user pressing Connect.
- **MCP language changes no longer fought by the settings panel (M-9):**
  the language dropdown now syncs from the config; `i18n::set` has a single
  writer.
- **Headless supervisor / connect-loop restart-flag race resolved (M-10):**
  the connect loop is the sole consumer of `take_config_restart` while a
  bot thread lives; the supervisor consumes it only when no thread exists.
  This removes the double-session / server-kick loop.
- **UI preview rebuild keyed on `snapshot_seq` (M-11):** two snapshot
  builds in the same second that change only block types now refresh the
  texture.
- **World-view heading arrow correct for non-zero yaw (M-12):** the
  renderer converts the stored degrees to radians before `sin` / `cos`
  (previously the yaw=0 test coincidence made the bug invisible).
- **`find_standable_neighbor` never returns fluid floors or positions above
  y=320 (M-13/M-14):** water/lava no longer count as standable floors and
  y+1 candidates are clipped to the world height, so the bot no longer
  pathfinds onto a lake or out of the world.
- **Wrong-tool mining-time penalty only for blocks that truly require a
  tool (L-1):** `calculate_mine_time` no longer applies the 5x wrong-tool
  penalty to blocks vanilla can mine by hand (`dirt`, `sand`, wood, wool),
  which used to estimate 3.75 s for a 0.75 s break.
- **Snapshots no longer contain air entries (L-4):** the dirty-block
  single-read path filters air like the dirty-chunk scan, so a broken block
  disappears from the snapshot (and `block_index`) instead of lingering as
  air.
- **`execute_command` trims whitespace before prepending `/` (L-5):**
  `" gamemode creative"` no longer becomes the unknown command
  `"/ gamemode creative"`.
- **`give_item` propagates executor failure instead of claiming success
  (L-6):** a `/give` the server accepts but does not honour no longer
  reports "Gave N x ..."; the `BotResult`'s `success`/feedback is surfaced.
- **`give_item` strips the `minecraft:` namespace in the swap fallback
  (L-7):** `minecraft:water_bucket` now matches the inventory's
  `water_bucket` instead of failing `InvalidParams`.
- **`walk_direction` rejects up/down at the MCP layer (L-8):** the
  validation now matches the documented contract instead of dispatching a
  command the executor rejects.
- **"no container open" returns `ContainerNotOpen` instead of
  `InvalidParams` (L-9):** see the new error code above.
- **Env-var config validates string fields per-field (L-11):** one bad
  `MINECRAFT_MCP_MCP_ADDRESS` no longer discards the entire environment
  configuration.
- **`attack_entity` bounds-checks `entity_id` before the snapshot scan
  (L-14):** out-of-range ids fail fast with `InvalidParams` instead of a
  full snapshot scan.
- **`act` no longer deep-clones the snapshot (L-15):** the per-call
  `Arc::make_mut` full-snapshot clone is replaced by a local `SelfPlayer`
  override.
- **`get_server_info` / `give_item` commands probe bounded to 3 s (L-18):**
  the live `/seed` probe uses `send_command_with_timeout`, so a busy
  executor can no longer stall these tools for the full command timeout.
- **Stale config-restart flag discarded at connect entry (L-22):** an
  explicit Connect after an offline `update_settings` no longer turns the
  next manual Disconnect into a surprise reconnect.
- **Activity probes immune to wall-clock jumps (L-23):** monotonic stamps
  (see Added) keep the snapshot-interval and idle-watchdog decisions
  correct across NTP adjustments.
- **`update_settings` commits the validated candidate:** the P3 refactor
  moved the `i18n::set` side effect after `validate()` but accidentally
  dropped the `SharedState::update_config` write, so the tool replied with
  the `applied` fields while the live config stayed unchanged. The
  validated candidate is now written back to the live config before any
  reconnect/restart side effect.
- **MCP Config panel rewrites wildcard bind addresses in the client URL:**
  binding to `0.0.0.0` / `::` now generates `http://127.0.0.1:...` /
  `http://[::1]:...` client URLs instead of the unconnectable
  `http://0.0.0.0:...` / `http://[::]:...` forms.

### Removed

- **Deprecated `find_best_tool_in_inventory` shim:** zero remaining callers
  (audit L-25).
- **Production-dead `WorldSnapshot::blocks_in_radius` /
  `entities_in_radius` helpers:** test-only Euclidean filters (audit L-26).

### CI

- **Pre-release download links use the derived version tag (R-1):** release
  notes on the pre-release channel previously used `GITHUB_REF_NAME`
  ("release"), so every download link in the body 404'd; the body now uses
  `steps.version.outputs.tag`.
- **Release-branch pushes with a suffix-less version are refused (R-2):** a
  guard step fails the workflow when `Cargo.toml` on `release` lacks an
  `-rc.N` suffix, preventing a premature `vX.Y.Z` prerelease that would
  block the stable release forever.
- **Docs site builds via `npm ci` against the committed lockfile (R-3):**
  `bun install` (which ignores `package-lock.json`) is replaced by
  `npm ci`; `actions/configure-pages` bumped to v5; leftover VitePress
  template comments removed.
- **Lint gate covers rustdoc + doctests and stops hiding failures:** the
  develop lint job now runs `cargo doc --locked --no-deps` (kept at zero
  warnings by the `[lints.rustdoc]` deny baseline), runs tests with
  `--no-fail-fast` so one broken target no longer masks the results of the
  remaining ones, and covers doctests explicitly via `cargo test --doc`
  (they are not part of `--all-targets`).
- **Weekly supply-chain audit (`audit.yml`):** new workflow runs
  `cargo audit` over every committed lockfile (root plus vendored
  `patches/rmcp` / `patches/rsa`) on Cargo.lock changes, a weekly schedule,
  or manual dispatch. Blocking gate: the first run's findings were triaged —
  `webbrowser` bumped to 1.2.4 (RUSTSEC-2026-0257) and `event-listener` to
  5.4.2 (RUSTSEC-2026-0221); the unfixable remainder (hickory-proto pinned
  by azalea, quick-xml pinned by wayland-scanner, unmaintained paste /
  ttf-parser) is documented per-entry in the new `.cargo/audit.toml` (the
  only location cargo-audit 0.22 discovers). Each
  audit leg runs in its lockfile's directory so per-directory triage
  configs stay isolated.
- **Nightly toolchain pinned to a DATE (`nightly-2026-05-28`):** a bare
  `nightly` let CI pick up the 2026-08-21 upstream compiler, which breaks
  `azalea-core` 0.15.1 compilation (E0284, const-generic inference in
  `FixedBitSet`) — nothing this repo can patch. The pin makes CI builds
  reproducible; both workflows now install via
  `actions-rust-lang/setup-rust-toolchain@v1`, which honors
  `rust-toolchain.toml` (the previous `dtolnay/rust-toolchain@nightly`
  always forced the latest nightly via `RUSTUP_TOOLCHAIN`, ignoring the
  pin). Bumping the date is part of dependency-upgrade work.

## [1.3.1] - 2026-08-16

### Added

- **Headless idle watchdog for stdio sessions:** a `--headless --stdio`
  process now shuts itself down after 10 minutes with no bot command
  dispatched (measured via the existing command-activity timestamp). This
  covers the lingering-process failure on Windows where stdin EOF never
  arrives (inherited console handles have no EOF; a pipe EOF requires every
  write end to be closed) and the client host abandons the session without
  closing the pipe.
- **Stdio Ctrl+C shutdown:** `serve_stdio` now races `tokio::signal::ctrl_c()`
  against the transport/shutdown-token futures (the HTTP path already had
  this), so a terminal Ctrl+C exits the process cleanly in every mode.
- **`get_world_view` / `get_bot_status` report a normalized `yaw`:** the
  raw look angle could grow unboundedly as the bot keeps turning (observed
  `-767.1°` in the annotation); it is now folded into Minecraft's
  `[-180, 180)` range at the snapshot write point.
- **Environment-variable configuration (config file removed):** settings are
  now read exclusively from `MINECRAFT_MCP_*` environment variables
  (12-factor style, like cargo); the `config.json` file, `--config <path>`
  flag and the `dirs` dependency were removed. Malformed values warn and
  keep the default. `MINECRAFT_MCP_TOKEN` is the only way to pin the MCP
  bearer token. `update_settings` / the UI panel now apply to the running
  process only.
- **`fly_timeout_secs` (default 60):** long `fly_to` flights no longer
  share the 30 s `command_timeout_secs`; the command envelope and the
  executor's goto margin honour the fly timeout for `FlyTo`.
- **Idle snapshot relaxation:** the snapshot rebuild interval relaxes to at
  least 5000 ms while no bot command has been dispatched for 3 s, cutting
  the snapshot cost of a parked bot by an order of magnitude (force-refresh
  paths are unaffected).
- **Coloured terminal output (cargo-style):** the `ansi` feature is now
  enabled, so ERROR lines render red, WARN yellow, INFO green, DEBUG blue —
  including when stderr is piped through `bunx`. The standard
  `NO_COLOR` environment variable disables colours.

### Changed

- **`break_block` defaults to the compound mine flow:** `use_best_tool`
  now defaults to `true`, so a bare `break_block` call approaches the
  block, picks the right tool (clear error when missing, e.g. shovel for
  grass), mines and verifies — the same behaviour as `act(Mine)`. The
  result carries `action_result` / `reason` / `self_info` (position).
  Set `use_best_tool=false` for the raw single-packet break.
- **`use_item_on_block` no longer fakes success:** new optional `face`
  argument (up/down/north/south/east/west, default up) selects the cell the
  placement lands in. The executor pre-checks the target cell is empty,
  auto-approaches when out of interaction range (4.5 blocks), and after the
  click polls the snapshot until the effect cell turns non-air (server-side
  confirmation). A rejected interaction returns an explicit
  `success:false` result instead of "success with no world change".
- **`get_nearby_blocks` response is now an object** (breaking wire
  change): `{blocks, count, total_matched, truncated, top_only}`. New
  `top_only` (highest block per column) and `max_blocks` (default 500)
  parameters cap the payload — the historical ~340 KB response for a flat
  radius-16 world collapses to a single surface layer.
- **Movement results carry the bot's position:** `collect_items` and
  `attack_entity` now include `position` in `data` so callers can detect
  the drift caused by auto-approach / item-walking without a separate
  `get_self_info` round-trip.
- **`walk_direction` reports the end position:** the result `data` now
  carries `position` (live player-position read preferred, snapshot
  fallback) so callers can tell where a successful walk actually ended.
- **`get_chat_history` documentation matches the 50-message retention cap:**
  the tool description and the desktop UI chat-log label now say 50 (the
  queue already retains 50 after the 2026-08 round-2 cursor fix) instead of
  the stale "up to 10".
- **`give_item` tool description updated:** the MCP tools/list description
  no longer calls the tool a smoke-test fallback and documents that rejected
  `/give` commands (e.g. an unknown item id) return `command_rejected`.
- **Help mode never claims to start the MCP server:** the "Minecraft MCP
  server starting" log line moved after mode resolution; a bare
  `minecraft-mcp-rs` invocation prints help and exits without starting the
  server.
- **`get_settings` no longer reports `config_path`** (breaking wire
  change) and `update_settings` no longer persists to disk.
- **MCP `instructions` advertise the supported Minecraft version:** Java
  Edition 1.21.11 (the only version supported by azalea 0.15.1).

- **CLI migrated to clap:** `src/cli.rs` now parses `--headless` /
  `--gui` / `--stdio` / `--config <path>` with clap 4.6 (derive) instead
  of the hand-rolled parser. Flags and precedence are unchanged
  (`--headless` wins over `--gui`, `--stdio` alone implies headless,
  `--config` alone runs the GUI); `-h/--help` and the new
  `-V/--version` print to **stderr** and exit 0, usage errors print to
  stderr and exit 2 — stdout stays reserved for the MCP transport. Help text
  is now generated by clap from the flag doc comments.
- **CI split (compile-time budget):** the run-once checks (fmt → clippy →
  test) live on `develop` — a PR targeting develop runs lint ONLY (the
  multi-platform matrix build is skipped), and the develop push runs lint +
  the full dev matrix once per merged change. `release.yml` now performs
  ONLY `--release` builds + publish (the duplicate lint job was removed;
  `release`/npm-publish depend on `[build, mode]`), so the release and
  pre-release paths never re-compile the test target matrix.
- **`build.yml` runs on `develop` only:** the push trigger moved from
  `master` to `develop`, and the `pull_request` trigger is now scoped to
  PRs targeting `develop` — master is stable-only and covered by the
  release pipeline.

### Removed

- **`--config <path>` CLI flag** (configuration is environment-variable
  based now).
- **Config-file persistence** (`config.json` read/write, `config_path`,
  the `dirs` dependency).

### Fixed

- **`execute_command` rejection detection now scans every reply message:**
  Minecraft reports a rejected command as TWO System chat messages (the
  error title, e.g. `Unknown or incomplete command. See below for error`,
  plus the command echo with a `<--[HERE]` marker). The old newest-only
  selection returned fake `success:true` whenever the echo landed last.
  `rejection_feedback_after` now checks every System message in the
  feedback window and returns the newest one matching a rejection pattern.
- **`give_item` no longer misfires "Permission denied":** the gate trusted
  the cached snapshot's `commands_enabled`, which can be a stale
  `PermissionLevel` heuristic right after a reconnect. `give_item` now
  re-probes command availability live via `/seed` (the same single source
  of truth `get_server_info` uses) and rejects only when the probe itself
  confirms commands are unavailable; the probe is also cleared on
  disconnect so it never crosses sessions.
- **`give_item` no longer fakes success for unknown item ids:** Minecraft
  rejects `/give` with an invalid id via `Unknown item '...'` (followed by
  the keywordless `<--[HERE]` command echo). The command-rejection scan now
  matches `unknown item` / `no such item`, so `give_item` returns
  `command_rejected` instead of `Gave N x nonexistent_item`.
- **`act` results report the bot's live position:** `self_info.position`
  previously came from the throttled snapshot, which can lag a just-finished
  move by up to one interval (5 s when idle) — an LLM client misread a
  successful move as "did not arrive". The result now prefers a zero-wait
  live read of the player's position and falls back to the snapshot only
  when unavailable.
- **Fluid bucket placement fails with a targeted error:** azalea 0.15.1's
  fabricated interaction hit (block centre, fixed Up face) is rejected by
  the vanilla server for bucket `UseItemOn`, so water/lava buckets cannot
  be placed through `use_item_on_block`. The verification timeout now
  returns `success:false` with `reason: "bucket_placement_unsupported"`
  and the working `/setblock` / `/fill` alternative instead of a generic
  "interaction was likely rejected".
- **Stdio server exits when the client's pipe breaks:** a failed response
  write (EPIPE) previously only logged and the process kept waiting for an
  input EOF that may never arrive. The patched rmcp serve loop now treats a
  transport write failure as a shutdown reason (`QuitReason::TransportWriteError`),
  so a headless stdio process dies the moment its MCP client disappears.
- **World-view cache key now uses a monotonic snapshot revision:** the
  cache used the seconds-granularity `WorldSnapshot::timestamp`, so two
  500 ms snapshot builds in the same second could share a timestamp and
  `get_world_view` returned a stale PNG. `WorldSnapshot` now carries an
  internal `snapshot_seq` (serde-skipped) that `SharedState` increments on
  every store; the cache key is `(snapshot_seq, radius, scale)`.
- **Semantically invalid environment values now fall back per-field:**
  `MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS=0`,
  `MINECRAFT_MCP_COMMAND_TIMEOUT_SECS=0`, `MINECRAFT_MCP_CHUNK_SCAN_RADIUS=0`
  and similar zero/out-of-range values previously parsed successfully and
  wedged the runtime (per-tick snapshots, instant command timeouts, no chunk
  scanning). `from_env` now validates each numeric value and falls back to
  the default with a warning; `main` runs a final `validate()` gate and falls
  back to full defaults if anything still slips through.
- **Snapshot `blocks` are now bounded by a retention radius:** old blocks
  outside `max(chunk_scan_radius, 8)` chunks from the player are pruned on
  every build, so a long-lived bot no longer accumulates every chunk it ever
  walked through (unbounded memory + per-tick clone/index rebuild).
- **`Act(Fly)` in non-creative mode now fails with `PermissionDenied`:**
  the executor previously returned `success:true, reached:false` when the
  bot was not in Creative, which contradicted the MCP `fly_to` gate and the
  "success means reached" rule.
- **`collect_items` only matches real dropped-item entities:** the old
  `contains("item") && !contains("frame")` filter also matched
  `item_display`; it now matches exactly `item` / `item_entity`.
- **`use_item_on_block` validates the effect cell too:** `face=up` at
  y=320 (effect y=321) was accepted and later timed out as a fake failure;
  it is now rejected at the MCP and executor validation gates.
- **`get_nearby_blocks` enforces `max_blocks` at runtime:** values outside
  `1..=10000` now return `InvalidParams` instead of silently truncating or
  trying to return everything.
- **`execute_command` rejection detection survives a full chat queue:**
  the feedback diff used the deque length as its baseline, so once the
  queue hit its cap every new message looked "before the baseline" and
  rejected commands were reported as successes (real sessions always had a
  full queue; unit tests never did). Chat messages now carry monotonic
  sequence numbers and the baseline is a cursor, so the scan is correct
  even when the deque is full. The chat cap was raised from 10 to 50.
- **`teleport` actually teleports (via `/tp`):** it previously mutated the
  local ECS `Position` component and reported success, but the server
  re-syncs the authoritative position every tick, so the bot never moved.
  It now sends `/tp x y z` and verifies the server reply — a rejected
  command (no OP) surfaces `CommandRejected` instead of a fake success.
  `fly_to`'s vertical landing leg uses the same server-authoritative
  teleport instead of the local position mutation.
- **`get_world_view` annotation counts are view-scoped:** `block_count` /
  `entity_count` reported the whole snapshot (hundreds of thousands of
  blocks for a 9-block viewport). The renderer now returns the distinct
  visible block columns and in-radius entities it actually drew, and the
  cache stores them so a cache hit returns the same numbers.
- **`place_block` names the placed block:** the result message previously
  read `Placed 3 at ...` (the hotbar slot). The MCP layer now resolves the
  item id from the inventory snapshot and reports `Placed stone at ...`;
  an empty slot is reported honestly as `(empty slot)`.
- **Headless idle watchdog keys on MCP requests, not bot commands:** a
  client host that spawns per-session connections and sends
  initialize/list_tools without ever dispatching a bot command (e.g. ZCode
  probe connections) was killed after 600 s, surfacing as "MCP server
  connection closed unexpectedly". `initialize` / `ping` / `list_tools` /
  `call_tool` now stamp MCP-request activity and the watchdog uses that.
- **`act` with `perception_radius=0` returns no nearby context:** the
  `<= 0` filter previously kept entities/blocks sharing the player's own
  cell, contradicting the documented "0 returns no nearby context at all".

## [1.3.0] - 2026-08-15

### Added

- **`act` payload trimming (`perception_radius`):** the unified act tool
  used to always return nearby blocks/entities at the configured
  `block_perception_radius` (default 32 — over 1 MB of JSON per call).
  `ActInput.perception_radius` (0..=32, `None` = configured default) now
  bounds the payload per call; `0` strips the nearby context entirely.
  Wired through as `BotCommand::Act(ActAction, Option<u32>)`.
- **`get_hotbar`:** explicit view of the 9 hotbar slots (0-8, empty slots
  as `null`) plus `held_item_slot` — the one-place answer to the slot
  layout that `set_hotbar_item` / `equip_tool` / `drop_item` operate on.
- **`get_bot_status`:** cheap polling endpoint for long-running operations
  (`fly_to`, mining, `collect_items`): `connected`, `bot_busy`, block +
  precise position, `yaw`, vitals, snapshot age. Reads the cached snapshot
  by default and reports `connected:false` while offline instead of erroring.
- **`give_item`:** the smoke-test command fallback packaged as a standard
  tool — `/give <bot> <item> <count>`, then `/item replace entity <bot>
  hotbar.<slot> with <item> <count>` for `target=hotbar`, falling back to
  the swap-click `set_hotbar_item` move when the server rejects
  `/item replace`. Requires server commands (OP).
- **Smoke-test skill:** `skills/minecraft-mcp-smoke-test/SKILL.md` codifies
  the full regression chain (connect → query → move → mine/place/use →
  combat → container → command → chat) with the chat-report protocol
  (`send_chat("[SMOKE] ...")` per category, `get_chat_history` as the
  assertion log).
- **Pre-release channel on the `release` branch:** pushing to `release`
  now automatically builds all platforms and publishes a **pre-release**
  (GitHub Release marked `prerelease`, never "Latest", + npm dist-tag
  `next`); the tag is derived from `Cargo.toml` (`v` + version, which
  carries an `-rc.N` suffix). Re-pushes with an unchanged version are
  idempotent (existing release / published npm versions are skipped).

### Changed

- **`smart_move` retries once on transient obstacles:** a first attempt that
  stops short (pathfinder error or "unreached") is retried once after a short
  pause before an obstacle is reported; results carry `retried`.
- **`attack_entity` auto-approaches moving targets:** a target farther than
  6 blocks is now approached (path to its last known position) and re-checked
  against a fresh snapshot before attacking, so a moving entity no longer
  forces a manual `move_to` dance — the caller still gets an honest
  `TooFar` when the entity keeps moving.
- **`use_item_on_block` reports the item actually used:** the result message
  now names the held item (e.g. `water_bucket`), so a wrong-slot
  interaction is visible instead of silently pouring nothing.
- **Branch model — `develop` / `release` / `master`:** feature PRs now
  target `develop`; `release` is the pre-release channel; `master` is
  stable-only — a `vX.Y.Z` tag on `master` publishes the stable release
  (GitHub "Latest Release" + npm `latest`). Documented in the new
  bilingual `CONTRIBUTING.md` and AGENTS.md. The old `release/1.2.0`
  branch was deleted (git forbids `release` and `release/<X.Y.Z>` to
  coexist) — version-specific prep branches must use another prefix.

### Removed

- **AtomGit `.gitcode/workflows/` mirror:** the three AtomGit pipeline
  files and all AtomGit references (README CI section, AGENTS notes) were
  removed — CI/CD runs exclusively on GitHub Actions. Historical changelog
  entries about AtomGit remain as records of past releases.

### Fixed

- **Inventory slot ordering (hotbar) — root cause of 4 broken tools:** azalea's
  `Menu::Player.inventory` (and the trailing 36 player slots of every other
  menu) is laid out in *protocol* order — **main inventory first (0-26),
  hotbar last (27-35)** — while the whole crate assumed hotbar-first (0-8).
  A new shared `canonical_player_inventory` / `canonical_inventory_slot`
  helper re-orders the 36 slots into the canonical hotbar-first order and
  reads them through `Menu::player_slots_range()`, so `get_inventory` /
  `get_self_info` now report hotbar slots at indices 0-8 and no longer
  return an empty list while a container is open. This one fix restores
  `set_hotbar_item`, `equip_tool`, and `drop_item` verification, which
  had all mis-mapped their slot indices.
- **`equip_tool` auto-moves tools from the main inventory:** a tool found
  only in the main inventory (slots 9-35) is now swapped into the first free
  hotbar slot and then selected, instead of failing with "move it to a hotbar
  slot first".
- **`fly_to` vertical movement:** azalea's pathfinder is ground-based and
  cannot change the player's Y beyond a 1-block jump. `fly_to` now splits
  the flight into a horizontal `goto` leg (target XZ at the current Y)
  followed by a direct position update for the vertical delta, so a target
  with a different Y is actually reached instead of timing out at 0
  displacement.

## [1.2.0] - 2026-08-13

### Fixed

- **`execute_command` no longer reports fake success:** the server rejects
  commands (e.g. `Incorrect argument for command ...`) via a chat message that
  was never correlated with the command. The executor now reads the server's
  system-message feedback after sending and surfaces a rejection as a new
  `CommandRejected` error (JSON-RPC -32009) carrying the verbatim feedback;
  accepted commands attach the server's reply (e.g. "Teleported X to ...") to
  the result. `get_server_info` uses the same mechanism to probe
  `commands_enabled` live via `/seed` (cached until `refresh=true`), so the
  field reflects whether commands *actually* work on cheat/plugin servers
  instead of the OP-level heuristic alone.
- **Stale-snapshot reads after actions:** `get_self_info` / `get_inventory`
  now accept `force=true` (default) and trigger an immediate snapshot rebuild
  before reading (3 s bounded), so a bot that just dropped an item / moved /
  teleported sees fresh state instead of the 500 ms-throttled snapshot.
  `collect_items` force-refreshes before scanning for dropped items.
- **`drop_item` verifies the drop actually landed:** the executor now reads
  the inventory before and after the click and reports `success: false` with a
  reason when the slot did not change (empty slot, or a container window
  making azalea drop the click packet).
- **Movement timeouts return structured partial results:** a stalled
  `move_to` / `smart_move` / `fly_to` no longer surfaces a bare
  `CommandTimeout` from the envelope — the executor stops the pathfinder and
  returns `{reason: "timeout", position, start, target, distance_moved}` so
  the MCP client can decide whether to retry or cancel.
- **`smart_move` obstacle field now carries coordinates:** the `obstacle`
  payload is `{block_type, x, y, z}` (or `null`), and the scan checks three Y
  layers plus a 3×3×3 neighbourhood fallback, so clients see what (and where)
  blocked the bot instead of `null`.
- **Dead mobs no longer linger in the entity list:** `collect_entities`
  skips entities whose health metadata is `<= 0`, so a killed chicken no
  longer shows up with `health: 0.0` until despawn. Item entities (no Health)
  are unaffected.

### Added

- **`set_hotbar_item(hotbar_slot, item_id, count)` MCP tool:** moves an
  existing inventory stack into a hotbar slot via a container swap-click — a
  reliable alternative to `/item replace`, whose syntax varies across
  servers. The item must already be in the inventory; it cannot conjure items.
- **`get_server_info` probe caching and `bot_busy` field.**
- **New `BotError::CommandRejected` variant with dedicated error code -32009**
  (`reason: command_rejected`, `retryable: true`, carries `command` +
  `feedback`).

### Changed

- **`smart_move` `obstacle` wire shape:** now an object `{block_type, x, y,
  z}` instead of a bare string (breaking change for clients reading the old
  shape).
- **`get_self_info` / `get_inventory` take a `force` parameter** (default
  `true`) — existing callers see identical output.

## [1.1.4] - 2026-08-12

### Fixed

- **Headless mode with the HTTP transport no longer hangs on exit:** the
  process could previously only be killed with Ctrl+C after `--headless`
  (without `--stdio`) bound the default HTTP transport, because the shutdown
  token was triggered only *after* `serve_http` returned, and `serve_http`
  only returns when the token fires. The HTTP server's graceful shutdown now
  races the shutdown token against an OS `SIGINT` (`shutdown_signal`), so
  Ctrl+C drains in-flight requests and exits cleanly in every mode.
- **"Command Stats" panel now reports real numbers:** the processed /
  succeeded / failed counters were never incremented by production code
  (only by unit tests), so the UI always showed 0/0/0/0%. `dispatch` now
  records every command that reaches the executor — including compound-op
  sub-commands and validation/offline rejections.
- **World-view preview no longer shows a stale frame after disconnect:** the
  cached render was never cleared on disconnect, so the preview panel kept
  displaying a frozen screenshot while the bot was offline. Disconnects now
  clear the world-view cache, and a missing cache forces the preview texture
  to be dropped.
- **`attack_entity` rejects targets missing from the world snapshot:** the
  bot-side handler now mirrors the MCP layer's existence check and returns
  `InvalidParams` (previously the reach check was silently bypassed for
  entities absent from the snapshot).

### Performance

- **Snapshot path no longer deep-clones the world on every tick:** the
  per-tick builder carried the *entire* previous snapshot (including its
  `block_index` HashMap) into `SnapshotBuilder`, and the built snapshot was
  cloned once more just to return it. The builder now takes only the block
  list, and the built snapshot is moved into `SharedState` — the per-tick
  clone cost drops from O(total blocks) to O(dirty blocks + surviving block
  strings).
- **Throttled ticks no longer spawn a wasted task:** the snapshot-interval
  throttle check now runs *before* spawning the build task, so ~18 of every
  20 ticks (500 ms interval against 20 TPS) skip the task entirely instead
  of spawning one that immediately returns.
- **Block names are cached per block state:** `block_state_to_name` resolves
  each distinct `BlockState` id once instead of paying `format!` +
  `to_snake_case` (two allocations) per block per tick during dirty-chunk
  scans.
- **`get_nearby_blocks` filter no longer allocates per block:** the
  case-insensitive substring match uses a new non-allocating ASCII helper
  (`utils::contains_ascii_case_insensitive`) instead of lowercasing every
  candidate block type.
- **Command dispatch no longer clones the command for the executor loop:**
  `BotCommandWithResponder` is destructured and the command moved into the
  handler.

### Changed

- **`i18n` moved to the crate root (`src/i18n/`):** the translation layer now
  lives at the top level instead of under `src/ui/`, removing the `mcp → ui`
  reverse dependency (`tools_settings` used `crate::ui::i18n`). No user-facing
  change — the `Language` serialization (`En`/`ZhCn`) is unchanged.
- **`anyhow` dependency removed:** the crate never used it; `eyre` remains
  for rare error-context needs.
- **Removed dead validation checks:** `AppConfig::validate` no longer checks
  `mc_port`/`mcp_port` against `65535` — both fields are `u16`, so the
  checks could never fire (the `== 0` lower-bound checks remain).

### Docs & housekeeping

- Docs-site GitHub links point to the real owner (`halfoffive`) instead of
  the `your-org` placeholder.
- README npm-publishing note now documents npm Trusted Publishing (OIDC) as
  the primary auth path, with `NPM_TOKEN` as the fallback.
- `deploy-docs.yml` upgraded `actions/checkout` from `v5` to `v6` to match
  the other workflows.
- Removed the unused `simulate_container_open` test stub and fixed two
  mojibake characters in `command_validate.rs` doc comments.

## [1.1.3] - 2026-08-10

### Added

- **`--gui` CLI flag:** the desktop UI can now be requested explicitly, and a
  bare invocation with no arguments prints the usage to stderr and exits 0
  instead of silently starting the UI. `--stdio` alone (without `--gui`) now
  implies headless server mode; when both are given, precedence is
  `--headless` > `--gui`.
- **`mcp_auth_enabled` HTTP Bearer-token switch (default OFF):** new config
  field gating the HTTP transport's token check. `validate()` rejects an
  empty token only when auth is enabled; an axum middleware gate
  (`is_request_authorized`, timing-safe compare, fail-closed when enabled)
  enforces the `Authorization` header at request time; `get_settings` /
  `update_settings` expose the flag (raw bool, not redacted); the Settings
  panel renders a "Require Bearer token" checkbox (HTTP transport only); the
  MCP Config panel omits the `Authorization` header from generated JSON when
  auth is off. Upgrade note: existing configs keep their persisted token, but
  auth is OFF by default after upgrading.
- **bunx variant of the MCP client config snippet:** the MCP Config panel and
  the docs now offer a `bunx minecraft-mcp-rs@1.1.3 --headless --stdio`
  snippet alongside the existing `npx` one.

### Changed

- **Release builds keep the Windows console:** the `windows_subsystem`
  attribute was removed, so release builds no longer hide the console window;
  `tracing` logs are visible on startup in every build configuration.
- **Version-pinned client snippets:** documentation and the UI MCP Config
  panel now pin an exact release (`minecraft-mcp-rs@1.1.3`). The UI snippet
  derives the version from `env!("CARGO_PKG_VERSION")` so it never goes stale;
  markdown docs use a literal `@1.1.3` (bump it whenever the Cargo/npm version
  changes).
- **Single-Minecraft-version notice + compatibility table:** the README,
  getting-started guides, and npm install docs now state that **only
  Minecraft Java Edition 1.21.11** is supported (no multi-version support) and
  include a server-version → tool-version table (1.21.11 ↔ 1.1.3) so users
  pick the correct release for their server.
- **CI gate order (fmt → clippy → test → build all platforms → release → npm):** the `lint` job now runs `cargo fmt --check` first, then `cargo clippy --locked --all-targets -- -D warnings`, then `cargo test --locked --all-targets`; the multi-platform `build` job depends on `lint` (`needs: lint`), and `release`/`npm-publish` depend on `[build, lint]` in `.github/workflows/` and mirrored in `.gitcode/workflows/`. npm publishing skips already-published versions.

### Fixed

- **Wrong Minecraft port no longer hangs (audit verdict):** the first
  connection attempt retries at most 3 times, then fail-fasts with the error
  surfaced in the Status panel; clicking Disconnect during a connect attempt
  aborts it immediately (`tokio::select!` + cancellation token + ECS
  `AppExit`); HTTP bind failures surface via `McpServerStatus::Failed` with
  red Status-panel text. See `src/bot/connection.rs:210-221,254-297` and
  `src/state.rs:379-392`.

## [1.1.2] - 2026-08-09

### Changed

- **npm platform packages moved under the `@minecraft-mcp-rs` org:** the five
  binary platform packages are published as
  `@minecraft-mcp-rs/minecraft-mcp-rs-{windows-x64,windows-arm64,darwin-arm64,linux-x64,linux-arm64}`
  (owned by the npm org). The main launcher stays unscoped as
  `minecraft-mcp-rs`; its `optionalDependencies` and the bin shim's
  `PLATFORMS` map now point at the scoped platform packages.
- **CI publishes via npm Trusted Publishing:** the `npm-publish` job's
  `npm publish --provenance` now authenticates through the GitHub OIDC token
  (per-package Trusted Publisher configured on npmjs.com) — no `NPM_TOKEN`
  secret needed; published versions carry provenance attestations.

## [1.1.1] - 2026-08-08

### Fixed

- **npm main package version bump:** `minecraft-mcp-rs@1.1.1` was published to
  work around npm's rule that an unpublished version number cannot be reused
  (the `1.1.0` version was deleted from the registry during the org migration
  and is permanently locked for this package name). No code changes in 1.1.1 —
  the binary content is identical to 1.1.0; the platform packages were already
  republished under the org scope at 1.1.0.

## [1.1.0] - 2026-08-08

### Added

- **Settings MCP tools:** `get_settings` (full config, MCP token redacted to
  `"***"`, plus runtime status), `update_settings` (partial update, validated
  and persisted to the config file before applying; changing
  `mc_address`/`mc_port`/`ai_username` triggers an automatic reconnect when
  connected, `mcp_transport`/`mcp_address`/`mcp_port` take effect on process
  restart), `connect_bot`, and `disconnect_bot`. All four work while the bot
  is offline. The `McpBotServer` now carries the command-receiver slot so
  `connect_bot` can spawn connections directly.
- **Config file persistence:** `AppConfig` is loaded from and saved to
  `config.json` in the OS config directory (`%APPDATA%\minecraft-mcp-rs\` on
  Windows, `~/.config/minecraft-mcp-rs/` on Linux, `~/Library/Application
  Support/minecraft-mcp-rs/` on macOS). Atomic write (temp file + rename,
  `0600` on Unix). `mcp_token` is persisted; it is redacted in every tool
  response.
- **Headless mode + CLI flags:** `--headless` (no desktop window; auto-connect
  on startup via a supervisor thread; process exits when the MCP transport
  closes), `--stdio` (force stdio transport), `--config <path>`, `-h`/`--help`.
  Manual argument parsing in `src/cli.rs` — no new dependency.
- **npm distribution:** `npm/minecraft-mcp-rs` (JS bin shim,
  `optionalDependencies` on five platform packages) +
  `minecraft-mcp-rs-{windows-x64,windows-arm64,darwin-arm64,linux-x64,linux-arm64}`
  carrying the stripped release binaries. Usage:
  `npx minecraft-mcp-rs --headless --stdio`. Root `LICENSE` (MIT) added.
- **CI npm-publish job:** appended to `release.yml` (triggered by the same
  `v*` tags plus `workflow_dispatch` with an optional `tag` input); reuses the
  build artifacts, publishes platform packages first then the main package,
  skips already-published versions, `--provenance --access public`, auth via
  the `NPM_TOKEN` secret (fail-fast if missing).
- **Error-contract table:** every `BotError` variant now maps to a distinct
  JSON-RPC code with structured `data` (`reason`, `retryable`, variant
  fields) — see `src/error.rs` and the README. `Offline` stays `-32000`
  (`bot_disconnected`).
- **MCP Config panel npx snippet:** the UI shows a second, copyable npx JSON
  block (stdio transport only) alongside the local-executable snippet.

### Changed

- **Connect loop hot-reloads config:** `ConnectionManager::connect` reads
  `ai_username`/`mc_address`/`mc_port`/`snapshot_interval_ms` and the
  reconnect backoff delays live from `SharedState` on every iteration instead
  of a frozen `self.config` clone, so agent-driven `update_settings` changes
  take effect on the next reconnect without restarting the process.
- **Entities rebuilt from the live ECS:** `SnapshotUpdater` now repopulates
  `entities` from azalea's entity storage on every snapshot rebuild instead
  of carrying over join-time player entries — `collect_items` and
  `get_nearby_entities` now actually see item drops and mobs. Event handlers
  no longer push entities into the snapshot.
- **`join_with_timeout` is `pub`:** the bounded-join helper moved from the UI
  crate to `src/bot/spawn.rs` and is shared by the headless supervisor and
  `MinecraftApp::drop`.
- **UI MCP Config panel** additionally renders the npx install variant.

### Fixed

- **Dead exponential-backoff reconnect:** `handle_spawn` latches
  `session_was_online` and `connect()` consumes it via
  `take_session_was_online()`, restoring the backoff branch that was
  unreachable because `handle_disconnect` clears the online flag before
  `ClientBuilder::start()` returns.
- **`Instant` underflow panic:** uptime under 1 hour no longer underflows.
- **`collect_items` never finding items:** entity list is now rebuilt from the
  live ECS (see Changed).
- **Dead `InventoryFull` guard:** the snapshot-based `inventory.len() >= 36`
  check in `take_from_container` (which could never fire with a container
  open) is replaced by a live inventory read.
- **Mine/place verification races:** verification now polls the snapshot with
  a bounded budget (`snapshot_interval_ms + 250 ms`) and treats air entries as
  "block gone/present" per the 1.0.7 air-in-snapshot semantics.
- **Walk up/down wrong error variant:** `Internal` → `InvalidParams`.
- **`slot:N` silent success:** malformed internal slot encodings now return
  `InvalidParams` instead of warn-and-continue.
- **`smart_move` success when blocked:** the blocked path reports
  `success: false` with a reason.
- **Non-constant-time bearer-token compare:** replaced with a
  constant-time XOR-accumulate comparison (length leakage accepted).

### Removed

- **`GameEvent` enum** (never constructed or consumed outside tests).
- **Unused `BotCommand::Query*` variants** (`QueryNearbyBlocks`,
  `QueryNearbyEntities`, `QuerySelfInfo`, `QueryChunkSummary`,
  `QueryServerInfo`, `QueryChatHistory`, `QueryWorldView`) and their handlers;
  `QueryInventory` remains. Variant count 34 → 27.

## [1.0.7] - 2026-07-25

### Added

- **IPv6 support in MCP config URL:** `format_host_for_url` wraps IPv6
  addresses in square brackets per RFC 3986 (e.g. `::1` → `[::1]`) when
  generating the MCP client config JSON, so clients like Claude Desktop /
  Cursor can connect to IPv6-bound MCP HTTP servers. IPv4 addresses and
  hostnames are emitted unchanged.

### Changed

- **`execute_place_block` hotbar-first item lookup:** the compound op now
  scans hotbar slots (0-8) first when locating the block item, only
  consulting the main inventory to produce a clear "move it to a hotbar
  slot (0-8)" error. Previously `position()` returned the first match
  across all 36 slots and relied on scan order to land on a valid hotbar
  slot.
- **Bearer auth empty-token semantics:** when the configured `mcp_token`
  is empty, authentication is now disabled entirely (all requests pass
  through). Previously an empty configured token made the HTTP server
  unreachable because no request could match an empty expected token.
  An empty `Bearer` header is now also rejected when a token is
  configured.

### Fixed

- **MCP parameter upper-bound validation:** `walk_direction` now rejects
  `distance > 1000`; `drop_item`, `take_from_container`, and
  `put_into_container` now reject `count > 64`; `take_from_container`
  and `put_into_container` now reject `slot > 53` at the MCP layer.
  Previously only the lower bound (0 / empty) was checked at the MCP
  layer; the upper bound lived solely in `validate_command`.
- **MCP parameter-vs-state validation order:** `handle_send_chat`,
  `handle_execute_command`, `handle_set_game_mode`, `handle_attack_entity`,
  and the four container handlers now validate parameters *before* the
  `is_online` / `check_container_open` gates, so a malformed request
  always yields `InvalidParams` rather than a misleading `Offline` /
  "no container open" error (extends the R-6 convention to the remaining
  handlers).
- **First-connect retry off-by-one:** `ConnectionManager::connect` now
  retries the first connection exactly `MAX_FIRST_CONNECT_RETRIES` times
  (changed `<=` to `<`), instead of one extra attempt.
- **`MinecraftApp::Drop` no longer clears the online flag:** the `online`
  flag is owned by the bot ECS (`handle_disconnect`); `Drop` previously
  called `set_online(false)` directly, racing the ECS teardown. Removed,
  matching the v1.0.5 Disconnect-button fix.
- **`connect_bot` thread-spawn panic:** `thread::Builder::spawn` failure
  in `connect_bot` is now handled gracefully — sets `last_error` and
  clears the `connecting` flag — instead of panicking via `.expect()`.
- **Air blocks now included in snapshots:** `build_snapshot_inner` no
  longer filters out `air` blocks. Previously `block_index` had no entry
  for air positions, so `find_standable_neighbor` (which checks "air
  block with solid block below") could not distinguish standable air
  from unloaded chunks, breaking placement targeting.
- **MCP HTTP server error surfacing:** `serve_http` now sets
  `McpServerStatus::Failed` and `last_error` when axum returns an error,
  so the failure is visible in the Status panel instead of only in the
  logs.
- **`connected_since` now updates on connect/disconnect:** `handle_spawn`
  and `handle_disconnect` call the new `SharedState::set_connected_since`
  to set/clear the connection timestamp. Previously the field was never
  written, so the Status panel's "connected since" indicator was always
  empty.
- **UI language persistence:** changing the Language dropdown now
  persists to `AppConfig` immediately via `update_config`, rather than
  waiting for the next Connect to apply.
- **UI preview Refresh gated on online state:** the world-view Refresh
  button is now disabled when the bot is offline, and a failed refresh
  clears the view cache.
- **CJK font doc comment:** the `fonts.rs` module doc now correctly
  states the CJK font is *appended* to `Proportional` (matching the
  v1.0.5 R-3 code change), not inserted at the front.
- **`EquipToolWithMaterial` test coverage:** the variant (added in v1.0.6)
  is now enumerated in `all_bot_commands()` and the variant-count
  assertion is bumped from 33 to 34, fixing the drift where the
  exhaustive test guard didn't actually cover the new variant.

## [1.0.6] - 2026-07-24

### Added

- **`SelfPlayer::position_precise` and `yaw` fields:** sub-block-precision
  player position (`[f64; 3]`) and horizontal look direction (`f32`, via
  `azalea::entity::LookDirection::y_rot()`). Both `#[serde(skip)]` so the
  JSON contract is unchanged. Populated by `SnapshotUpdater` for use by
  the top-down renderer.
- **World view cache:** `SharedState` gains `WorldViewCache` with
  `get_world_view_cache()` / `set_world_view_cache()` /
  `clear_world_view_cache()` accessors. Cache key is
  `(snapshot_timestamp, radius, scale)`. Single-entry, bounded memory.
- **Enhanced top-down renderer:** `render_topdown_enhanced` supports
  `scale` (1/2/4/8 pixels per block), Y-axis brightness modulation
  (higher blocks → brighter), sub-block-precision centre via
  `position_precise`, and a yaw heading arrow at the player marker.
- **`get_world_view` multi-content response:** now returns
  `[image, text-annotation]` — the JSON annotation includes centre
  coords, radius, scale, yaw, and timestamp. Added `scale` parameter
  to `GetWorldViewInput`.
- **Section-level chunk scanning:** `SnapshotUpdater` scans dirty chunks
  section by section (16×16×16), skipping entirely-air sections via
  `section.block_count == 0`. Reduces per-chunk `get_block_state` calls
  from 98304 to ~4096 for typical surface chunks.
- **UI error dismiss button:** red error banner in the status panel now
  has a "×" button that clears `last_error`.
- **UI connecting spinner:** an `egui::Spinner` appears next to the
  "Connecting…" label when the bot is in a connection attempt.
- **UI world-view preview panel:** a new collapsing section after Status
  shows the cached `get_world_view` PNG (decoded to an egui texture).
  Includes a "Refresh" button that clears the cache and re-renders at
  `radius=8, scale=2`.
- **`WorldViewCache` struct and accessors:** for caching the most recent
  `get_world_view` response. Stored in `SharedState::last_world_view`.
- **`rmcp` patch:** added `impl IntoContents for Vec<Content>` so the
  `#[tool]` macro accepts `Vec<Content>` return types for multi-content
  MCP tool responses.

### Changed

- `color_map` in `render.rs` expanded: added planks, glass, wool,
  concrete, terracotta, glazed terracotta, bricks, plants, functional
  blocks, nether, end, and containers. Over 400 new block-type mappings.
- `render_topdown` background changed from transparent (alpha=0) to
  opaque sky-blue (alpha=255 everywhere), fixing rendering quirks on
  MCP clients that display black for alpha=0 pixels.
- Block lookup in `render_topdown` uses a flat `Vec<Option<...>>`
  indexed by `px * size + py` instead of the previous `HashMap`,
  eliminating per-block hashing overhead on large snapshots (5000+
  blocks).
- `get_world_view` in `server.rs` now passes `input.scale` to the
  renderer and returns `Vec<Content>`.

## [1.0.5] - 2026-07-19

### Added

- **AtomGit Action pipelines:** added `.gitcode/workflows/` with three workflows
  mirroring the GitHub Actions setup and adapted to AtomGit platform
  constraints — `build.yml` (dev binary matrix build + lint/test), `release.yml`
  (`v*`-tag release build + packaging + publish), and `deploy-docs.yml`
  (VitePress site build, artifact upload only). Platform differences vs GitHub:
  workflow directory is `.gitcode/workflows/`; built-in actions have no version
  suffix (`uses: checkout` / `upload-artifact` / `download-artifact`); runner
  labels are three-segment lists (e.g. `[ubuntu-24, x64, small]`); Rust nightly
  is installed via `rustup toolchain install nightly`; context vars are
  `ATOMGIT_*` (`ATOMGIT_REF` / `ATOMGIT_REPOSITORY` / `ATOMGIT_TOKEN`); the
  matrix covers only `linux-x86_64` / `linux-aarch64` / `windows-x86_64`
  (AtomGit docs expose no macOS hosted runner and only `windows-2022` x64 —
  `macos-aarch64` and `windows-aarch64` need self-hosted runners and are out
  of v1's default matrix); v1 enables no cargo cache (AtomGit cache action
  name unconfirmed); `concurrency.exceed-action` is `IGNORE` for build and
  `QUEUE` for deploy-docs (AtomGit supports only IGNORE/QUEUE, no
  cancel-in-progress equivalent); the release publish step is a commented curl
  template because the AtomGit release API endpoint is unconfirmed — enable
  after platform confirmation (download-link `base_url` uses atomgit.com as a
  placeholder); `deploy-docs.yml` uploads the site as an artifact only (no
  GitHub Pages / AtomGit Pages deployment). Primary host remains GitHub; the
  AtomGit pipelines need no real-run verification (per user requirement) and
  are picked up automatically once the repo is mirrored/hosted on AtomGit.

### Fixed

- **Ancient debris harvest level:** corrected `ancient_debris` from harvest
  level 4 (netherite) to 3 (diamond+); a diamond pickaxe can now mine it,
  matching vanilla. Previously a diamond-only inventory was wrongly refused
  with `ToolNotFound`.
- **`equip_tool` material preference wired:** the `material_preference` MCP
  parameter is now honoured (previously echoed into the response but ignored).
  It maps to a minimum material tier via the new
  `BotCommand::EquipToolWithMaterial(ToolType, MaterialTier)`; an unknown tier
  returns `InvalidParams`. Requesting a tier higher than any tool in the
  inventory yields `ToolNotFound`.
- **Centralized command validation:** `CommandExecutor::dispatch` now runs
  `command_validate::validate_command` on every command, closing runtime gaps
  where MCP handlers omitted upper-bound checks (container `slot` > 53 /
  `count` > 64, `walk_direction` distance > 1000). `validate_command` was
  previously defined and tested but never invoked in production.
- **`find_obstacle_block` interpolation:** the SmartMove obstacle scan now
  interpolates both axes along the true line instead of stepping a 45°
  diagonal, so it no longer overshoots one axis or skips intermediate cells.
- **Docs tool name:** corrected `set_gamemode` to the actual registered tool
  name `set_game_mode` in the README and the English/Chinese tool references.
- **Block data sync:** added `ancient_debris` and `netherite_block` to
  `BLOCK_TO_TOOL_TYPE` and `BLOCK_HARDNESS`. Synchronised all three tables
  (TOOL_TYPE / HARDNESS / HARVEST_LEVEL) — 90+ missing block hardness values
  added, all blocks in TOOL_TYPE now have matching hardness entries.
- **Type conversion safety:** entity position in `handle_add_player` now uses
  `f64.round() as i32` instead of truncation cast; `MinecraftEntityId` uses
  `u32::try_from()` with fallback instead of bitcast.
- **Executor abort safety:** `handle_disconnect` now calls `JoinHandle::abort()`
  and yields 50 ms before reconnecting, giving `ReceiverLease` time to return
  the receiver to the slot. The mutex lock is released before `.await` so the
  async handler's `Send` bound is satisfied.
- **Removed dead `INJECTION_READY` flag:** the guard flag was written on
  connect/disconnect but never read; its doc comment falsely claimed
  `BotState::default` consulted it. Deleted along with the now-unused
  `AtomicBool` import in `bot/events.rs`.
- **Removed dead code:** deleted unused `BotError::ConnectionFailed` variant,
  its Display impl, MCP error mapping, and associated test.
- **Redundant yield removed:** `ReceiverLease::take_with_retry` no longer calls
  `yield_now()` before `sleep(5ms)`.
- **MCP token serialization:** `mcp_token` is now marked `skip_serializing` so
  it is never written to serialised config output.
- **Bot connection thread panic resilience:** `Runtime::new()` failure no
  longer panics the connection thread. A `ClearGuard` RAII guard ensures
  `clear_connecting()` always runs, preventing the `bot_connecting` flag from
  being permanently stuck (which previously blocked all subsequent Connect
  attempts until process restart).
- **`walk_direction` integer overflow:** `distance as i32` has been replaced
  with `clamp_to_i32(distance)`, and all coordinate arithmetic now uses
  `saturating_add` / `saturating_mul`.  A malicious `distance > i32::MAX` no
  longer silently wraps to a negative offset (which made the bot walk in the
  opposite direction).
- **MCP tool radius runtime validation:** `get_nearby_blocks` and
  `get_nearby_entities` now reject `radius` outside 1..=100 at runtime,
  matching the `#[schemars(range)]` JSON Schema contract.  Previously an
  out-of-bounds radius silently clamped (or in the integer-overflow case,
  produced incorrect results).
- **`handle_collect_items` semantics:** variable renamed from `collected` to
  `visited`; the result message now reads "Visited N item drop location(s);
  auto-pickup expected on proximity".  `goto` success means the bot reached
  the position, not that the server processed the pickup — the old message
  was misleading to LLM consumers.
- **`handle_place_block` result message:** `"slot:N"` internal prefixes are
  stripped from the success message so the LLM sees a clean block type
  instead of an opaque hotbar index.
- **`get_nearby_blocks` filter performance:** `filter_type.to_lowercase()` is
  now pre-computed outside the block-scanning closure so it isn't re-allocated
  per block (up to thousands of allocations at radius 100).
- **Container tool validation order:** `count == 0` is now checked before
  `check_container_open` / `is_online` in both
  `handle_take_from_container` and `handle_put_into_container`, matching
  the convention in `tools_item.rs` (parameter errors before state errors).

### Changed

- **CJK font fallback strategy in egui:** the system CJK font is now
  *appended* to `Proportional` instead of *prepended*, so Latin glyphs
  consistently render with Ubuntu-Light and only non-Latin glyphs fall
  through to the system CJK font.
- **UI language sync optimisation:** `MinecraftApp` caches `last_language`
  and only re-reads `AppConfig::language` when the value actually differs,
  eliminating a per-frame `RwLock` acquisition.

## [1.0.4] - 2026-07-15

### Added

- **System locale auto-detection:** the UI now detects the OS locale via
  `sys-locale` and uses it as the default language on first launch.
  `AppConfig::language` still persists the user's explicit choice.

### Changed

- **CI uses dev profile for non-release builds:** `.github/workflows/build.yml`
  now runs `cargo build --locked --target <target>` (dev profile) instead of
  `--release`, so PR/push CI builds are faster and no longer trigger the
  release-only Windows console hiding. `release.yml` still uses `--release`
  with `strip` for published binaries.
- **CI deduplicates workflow runs:** `build.yml` now limits `push` triggers to
  the `master` branch (so internal PR branches only trigger the
  `pull_request` workflow), adds a top-level `concurrency` group with
  `cancel-in-progress: true` to cancel stale runs, and decouples the `lint`
  job from the `build` matrix so lint/test runs in parallel with the
  cross-platform builds.

### Fixed

- **BotError propagation through MCP tools:** all MCP tool handlers
  (`tools_chat`, `tools_block`, `tools_movement`, `tools_item`,
  `tools_container`, `tools_combat`, `tools_act`) now propagate the actual
  `BotError` from `send_command` instead of wrapping every failure in
  `InternalError`. This lets MCP clients distinguish offline, timeout,
  and validation errors.
- **Disconnect during connection attempt:** `ClientBuilder::start()` in
  `connection.rs` is now wrapped in `tokio::select!` with
  `cancel_token.cancelled()`, so clicking Disconnect while azalea is still
  trying to TCP-connect aborts the attempt immediately instead of waiting
  for the ~5 s timeout to expire.
- **Release console window:** `src/main.rs` now carries
  `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` so
  release builds on Windows no longer flash a black console window. Debug
  builds retain the console for diagnostics.
- **i18n missed strings:** the MCP server status line in the Status panel
  (`Running on <addr>` / `Running on stdio` / `Failed: <msg>` / `Stopped`),
  the non-loopback HTTP TLS warnings in both the Settings and MCP Config
  panels, and the transport `ComboBox` collapsed label are now routed
  through `TextKey` translations. Switching the UI language now updates all
  of them on the next frame instead of leaving hard-coded English behind.
- **i18n language sync:** the Settings panel now syncs the i18n language
  before rendering labels so the UI reflects the change immediately on the
  current frame.
- **egui repaint after connection state changes:** connection state
  transitions now request an egui repaint so the Status panel updates
  without waiting for the next user interaction.

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
