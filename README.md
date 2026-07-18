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
inventory management, combat, and chatting — as MCP tools over **stdio** or
**remote HTTP** (loopback-only, Bearer-token protected).

本项目将 LLM 客户端（Claude Desktop、Cursor 等）桥接到真实的 Minecraft 游戏世界。一个由 [azalea] 库驱动的 Rust 机器人会连接到你的 Minecraft 服务器，并将其移动、挖掘、建造、背包管理、战斗和聊天等能力作为 MCP 工具暴露出来，支持 **stdio** 或 **远程 HTTP**（仅限本地回环，受 Bearer Token 保护）两种传输方式。

The bot targets **Minecraft Java Edition 1.21.11** (via azalea 0.15.1).

该机器人目标版本为 **Minecraft Java Edition 1.21.11**（通过 azalea 0.15.1 实现）。

[azalea]: https://github.com/azalea-rs/azalea

## Features

- **30+ MCP tools** organized into 7 domains, plus a unified `act` tool

  提供 **30 余个 MCP 工具**，分为 7 个领域，外加统一的 `act` 工具。

- **Bilingual UI (English / 简体中文)** — switch languages at runtime in the
  Settings panel; CJK system fonts are auto-detected so Chinese renders
  out of the box

  **双语 UI（英文 / 简体中文）** —— 可在运行时通过“设置”面板切换语言；系统会自动检测 CJK 字体，开箱即用即可正常显示中文。

- **Live world state** — the bot periodically snapshots its surroundings into a
  thread-safe `SharedState` readable by all tools

  **实时世界状态** —— 机器人会定期将周围环境快照保存到线程安全的 `SharedState`，所有工具均可读取。

- **Remote MCP HTTP server** — loopback-only (`127.0.0.1`), Bearer-token
  protected; transport mode (stdio / HTTP) selectable in the UI

  **远程 MCP HTTP 服务** —— 仅限本地回环（`127.0.0.1`），受 Bearer Token 保护；stdio / HTTP 传输模式可在 UI 中选择。

- **AI vision for multimodal models** — `get_world_view` renders a top-down PNG
  of nearby blocks and returns it as base64

  **多模态模型的 AI 视觉** —— `get_world_view` 可渲染附近方块的俯视 PNG 图并以 base64 返回。

- **Smart movement & flight** — `smart_move` auto-jumps over 1-block gaps and
  stops on larger obstacles; `fly_to` flies to a target in creative mode

  **智能移动与飞行** —— `smart_move` 可自动跳过 1 格宽的缺口，遇到更大障碍会停下；`fly_to` 可在创造模式下飞向目标。

- **Desktop UI** (egui/eframe) — status panel with live stats, settings panel
  to configure connection, bot parameters, and MCP transport, plus an MCP
  Config panel that shows a copyable JSON config for Claude Desktop / Cursor

  **桌面 UI**（egui/eframe）—— 状态面板实时显示统计信息，设置面板可配置连接、机器人参数和 MCP 传输方式，另有 MCP 配置面板可生成可复制到 Claude Desktop / Cursor 的 JSON 配置。

- **Auto-reconnect** — exponential backoff on disconnect; the command executor
  is cleanly aborted and re-started on each reconnect via a `ReceiverLease`

  **自动重连** —— 断开连接时采用指数退避；每次重连都会通过 `ReceiverLease` 干净地中止并重新启动命令执行器。

- **Compound operations** — higher-level state machines (mine-and-collect)
  built on primitive commands

  **复合操作** —— 基于原始命令构建的更高层状态机（例如“挖掘并拾取”流水线）。

- **Thread-safe by design** — lock-free snapshots via `ArcSwap`, atomic flags,
  `RwLock` for config, `Mutex` for container handles

  **设计上线程安全** —— 通过 `ArcSwap` 实现无锁快照，原子标志控制状态，`RwLock` 保护配置，`Mutex` 保护容器句柄。

- **Dirty-region snapshot updates** — only changed blocks/chunks are
  recomputed between polling intervals

  **脏区快照更新** —— 仅重算两次轮询之间发生变化的方块 / 区块。

- **Configurable command timeout** — `command_timeout_secs` is honoured by
  the command channel (not just a UI field)

  **可配置的命令超时** —— `command_timeout_secs` 由命令通道真正遵守，而不仅是 UI 字段。

- **Honest error reporting** — `BotError::InvalidParams` maps to MCP
  `INVALID_PARAMS`; unbreakable blocks return `MiningInterrupted` instead of
  panicking; `set_game_mode` flags the OP requirement

  **诚实的错误报告** —— `BotError::InvalidParams` 会映射为 MCP 的 `INVALID_PARAMS`；无法破坏的方块返回 `MiningInterrupted` 而非 panic；`set_game_mode` 会提示需要 OP 权限。

## Tool Categories

| Category / 类别 | Tools / 工具 |
|-----------------|--------------|
| **Query / 查询** | `get_self_info`, `get_inventory`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **Movement / 移动** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **Block / 方块** | `break_block`, `place_block`, `use_item_on_block` |
| **Item / 物品** | `drop_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items` |
| **Container / 容器** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **Combat / 战斗** | `attack_entity`, `shield_block` |
| **Chat / 聊天** | `send_chat`, `execute_command`, `set_game_mode` |
| **Unified / 统一** | `act` — one tool that can move, smart-move, fly, mine, attack, or collect items and returns an environment snapshot |

上述工具按功能领域分类；`act` 是一个统一入口，能够根据传入参数执行移动、智能移动、飞行、挖掘、攻击或拾取物品，并返回环境快照。

## Documentation

A bilingual (English / 简体中文) documentation site built with VitePress is
available. After enabling GitHub Pages on the repository, it is served at
`https://<user>.github.io/minecraft-mcp-rs/`.

项目文档使用 VitePress 构建，提供英文 / 简体中文双语版本。开启仓库的 GitHub Pages 后，可通过 `https://<user>.github.io/minecraft-mcp-rs/` 访问。

To run the docs locally:

```bash
npm install
npm run docs:dev      # dev server at http://localhost:5173
npm run docs:build    # production build into docs/.vitepress/dist
```

本地运行方式与上述命令相同；安装依赖后，`npm run docs:dev` 启动开发服务器，`npm run docs:build` 生成生产构建。

## Language / 语言切换

The desktop UI supports **English** and **简体中文**. Pick a language from the
**Language** dropdown in the Settings panel — the change takes effect on the
next frame without reconnecting. On startup the app auto-detects the system's
default CJK font (Windows `msyh.ttc`, macOS `PingFang.ttc`, Linux Noto /
WenQuanYi) so Chinese text renders correctly without manual font setup.

桌面 UI 支持 **英文** 与 **简体中文**。在"设置"面板的"语言"下拉框中切换，下一帧即
生效，无需重连。启动时会自动探测系统默认中文字体（Windows `msyh.ttc`、macOS
`PingFang.ttc`、Linux Noto / 文泉驿），无需手动安装字体即可正常显示中文。

## Quick Start

### Prerequisites

- [Rust nightly](https://rustup.rs/) (pinned in `rust-toolchain.toml`, edition 2024; azalea 0.15.1 requires nightly)
- A Minecraft Java Edition 1.21.11 server (local or remote)
- An MCP client (Claude Desktop, Cursor, or any MCP-compatible LLM host)

前置条件：需要安装 [Rust nightly](https://rustup.rs/)（`rust-toolchain.toml` 已固定，edition 2024；azalea 0.15.1 要求 nightly）、一个 Minecraft Java Edition 1.21.11 服务器（本地或远程），以及一个 MCP 客户端（Claude Desktop、Cursor 或任何兼容 MCP 的 LLM 宿主）。

### Build

```bash
cargo build
```

构建项目。

### Run

```bash
cargo run
```

This starts both the MCP server and the egui desktop UI. Choose the MCP
transport in the Settings panel:

- **stdio** — the MCP server listens on stdin/stdout (default for Claude
  Desktop / Cursor).
- **HTTP** — the MCP server binds to `127.0.0.1` only; set the port and
  Bearer token (a random UUID v4 is generated on each `AppConfig::default()`;
  override it in the Settings panel). The MCP Config panel generates the
  matching JSON config for copying into your MCP client.

运行后会同时启动 MCP 服务器与 egui 桌面 UI。在“设置”面板中选择 MCP 传输方式：

- **stdio** —— MCP 服务器监听标准输入 / 输出（Claude Desktop / Cursor 的默认方式）。
- **HTTP** —— MCP 服务器仅绑定到 `127.0.0.1`；可在 UI 中设置端口与 Bearer Token（每次 `AppConfig::default()` 时随机生成 UUID v4，可在设置面板中覆盖）。MCP 配置面板会生成对应的 JSON 配置，可复制到你的 MCP 客户端中。

By default the bot tries to connect to `127.0.0.1:25565` as `AI_Bot`. Tweak
settings in the UI panel or via environment before startup (see Configuration).

默认情况下，机器人会尝试以 `AI_Bot` 身份连接到 `127.0.0.1:25565`。可以在 UI 面板或启动前通过环境变量调整设置（详见 Configuration 章节）。

### Test

```bash
cargo test                # all tests
cargo test --lib          # unit tests only
cargo test --test integration  # mock-based integration tests
cargo test --test proptest     # property-based tests
```

运行测试。

## Dependency Patches

The repository includes two small, tracked patches under `patches/` to resolve
upstream dependency conflicts that block a clean build:

- `patches/rmcp` — based on `rmcp 1.8.0` (Apache-2.0). Downgrades the optional
  `rand` dependency from `0.10` to `0.9` so it does not conflict with
  `azalea-crypto`'s pinned `rand_core = "=0.10.0-rc-5"`.
- `patches/rsa` — based on `rsa 0.10.0-rc.13` (MIT OR Apache-2.0). Adjusts
  `src/encoding.rs` to use the tuple variant `pkcs8::Error::KeyMalformed(...)`
  required by `pkcs8 0.11.0`.

These directories are committed to git and are referenced by the
`[patch.crates-io]` section in `Cargo.toml`. Do not add them to `.gitignore`;
CI and other clones need them on disk to resolve dependencies.

这些目录已提交到 git，并由 `Cargo.toml` 中的 `[patch.crates-io]` 节引用。请勿将它们加入 `.gitignore`；CI 及其他克隆仓库需要这些文件在本地才能正确解析依赖。

## Configuration

All settings have sensible defaults and can be changed at runtime through the
egui settings panel (fully editable — text inputs for strings, DragValue
sliders for numeric fields). After editing, click **Connect** to apply the
settings and spawn the bot connection on a dedicated background thread.

所有设置均有合理默认值，可在运行时通过 egui 设置面板修改（完全可编辑——字符串使用文本输入框，数值字段使用 DragValue 滑块）。编辑完成后点击 **Connect**，设置即会生效，并在专用后台线程上启动机器人连接。

| Field / 字段 | Default / 默认值 | Description / 说明 |
|--------------|------------------|--------------------|
| `mc_address` | `127.0.0.1` | Minecraft server address / Minecraft 服务器地址 |
| `mc_port` | `25565` | Minecraft server port / Minecraft 服务器端口 |
| `ai_username` | `AI_Bot` | Bot in-game username / 机器人游戏内用户名 |
| `mcp_transport` | `Http` | MCP transport: `Stdio` or `Http` / MCP 传输方式：`Stdio` 或 `Http` |
| `mcp_address` | `127.0.0.1` | MCP HTTP bind address (loopback only) / MCP HTTP 绑定地址（仅本地回环） |
| `mcp_port` | `3000` | MCP HTTP port / MCP HTTP 端口 |
| `mcp_token` | random UUID v4 | Bearer token for HTTP transport (generated on each `AppConfig::default()`) / HTTP 传输的 Bearer Token（每次 `AppConfig::default()` 时随机生成） |
| `language` | `En` | UI language: `En` or `ZhCn` / UI 语言：`En` 或 `ZhCn` |
| `chunk_scan_radius` | `8` | Chunks to scan (1–16) / 扫描区块半径（1–16） |
| `block_perception_radius` | `32` | Block awareness range (8–64) / 方块感知范围（8–64） |
| `snapshot_interval_ms` | `500` | World snapshot interval / 世界快照间隔（毫秒） |
| `reconnect_initial_delay_ms` | `5000` | Initial reconnect backoff / 初始重连退避（毫秒） |
| `reconnect_max_delay_ms` | `60000` | Maximum reconnect backoff / 最大重连退避（毫秒） |
| `command_timeout_secs` | `30` | Bot command timeout / 机器人命令超时（秒） |

数值型字段均可在 UI 中通过滑块或键盘输入调整；修改后需点击 **Connect** 才会应用到机器人连接。

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
│   MCP Server (rmcp, stdio or HTTP transport)    │
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

机器人在独立的操作系统后台线程上运行，拥有自己的 tokio 运行时；UI 运行在主线程。两者通过 `Arc<SharedState>`（无锁读取）和 `BotCommand` 通道（tokio mpsc + oneshot 响应）进行通信。

## Project Structure

```
src/
  types.rs            — Shared data types (BlockPos, BotCommand, ActAction, …)
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
  mcp/                — MCP server + 8 tool modules (incl. act, render)
  ui/                 — egui app shell, settings, status, mcp_config
tests/
  integration.rs      — Mock-based end-to-end tests
  proptest.rs         — Property-based tests
```

代码按“类型 → 逻辑 → 状态 → 机器人 → MCP → UI”分层，全部位于同一个 crate 中。`bot/` 负责机器人生命周期与事件处理，`mcp/` 暴露 MCP 工具，`ui/` 提供 egui 桌面界面。

## Development

### Logging

All log output goes to **stderr** only — stdout is reserved for MCP JSON-RPC
transport. Default filter: `minecraft_mcp_rs=debug, azalea=warn`.

所有日志输出仅写入 **stderr** —— stdout 保留给 MCP JSON-RPC 传输使用。默认过滤级别：`minecraft_mcp_rs=debug, azalea=warn`。

### Testing Conventions

- Unit tests live at the bottom of each source file in `#[cfg(test)] mod tests`

  单元测试位于每个源文件底部的 `#[cfg(test)] mod tests` 中。

- Integration tests in `tests/integration.rs` use mocks (no real MC server)

  `tests/integration.rs` 中的集成测试使用 mock，无需真实 Minecraft 服务器。

- Property tests in `tests/proptest.rs` use the `proptest` crate

  `tests/proptest.rs` 中的属性测试使用 `proptest` crate。

### Development hooks

Currently a no-op stub. Once the project ships pre-commit / commit-msg hooks
(planned: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --locked`, and a Conventional Commits linter), you will opt in
with:

```bash
bash scripts/install-hooks.sh
```

Until then this script does not touch `git config core.hooksPath` — running
`git config --get core.hooksPath` after `cargo build` should report an error,
confirming no global side effects are configured.

当前 `scripts/install-hooks.sh` 为占位实现。计划中会加入 pre-commit（`cargo fmt --check`、
`cargo clippy --all-targets -- -D warnings`、`cargo test --locked`）与 commit-msg
（Conventional Commits 检查）钩子，届时执行 `bash scripts/install-hooks.sh` 即可启用。
在那之前脚本不会修改 `git config core.hooksPath`，所以 `cargo build` 之后跑
`git config --get core.hooksPath` 应报错，表示没有任何全局副作用被设置。

## License

MIT

许可证：MIT
