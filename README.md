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

> ⚠ **Single Minecraft version / 仅支持单一 Minecraft 版本**
>
> This tool supports **only Minecraft Java Edition 1.21.11** — other Minecraft versions are **NOT** supported. There is no multi-version compatibility layer.
>
> 本工具**仅支持 Minecraft Java Edition 1.21.11**——其他 Minecraft 版本**均不支持**。本工具没有多版本兼容层。
>
> | Minecraft server version / MC 服务器版本 | minecraft-mcp-rs version / 工具版本 |
> |---|---|
> | 1.21.11 | 1.3.1 |
>
> Pick the minecraft-mcp-rs version that matches your Minecraft server, and re-check this table before every upgrade. / 请选择与你的 Minecraft 服务器版本匹配的 minecraft-mcp-rs 版本，并在每次升级前重新核对此表。

[azalea]: https://github.com/azalea-rs/azalea

## Features

- **30+ MCP tools** organized into 8 domains, plus a unified `act` tool

  提供 **30 余个 MCP 工具**，分为 8 个领域，外加统一的 `act` 工具。

- **Bilingual UI (English / 简体中文)** — switch languages at runtime in the
  Settings panel; CJK system fonts are auto-detected so Chinese renders
  out of the box

  **双语 UI（英文 / 简体中文）** —— 可在运行时通过“设置”面板切换语言；系统会自动检测 CJK 字体，开箱即用即可正常显示中文。

- **Live world state** — the bot periodically snapshots its surroundings into a
  thread-safe `SharedState` readable by all tools; the snapshot keeps only the
  blocks within `max(chunk_scan_radius, 8)` chunks of the player, so memory
  stays bounded on long walks

  **实时世界状态** —— 机器人会定期将周围环境快照保存到线程安全的 `SharedState`，所有工具均可读取；快照只保留玩家周围 `max(chunk_scan_radius, 8)` 个区块内的方块，长途移动时内存保持有界。

- **Remote MCP HTTP server** — loopback-only (`127.0.0.1`), optional
  Bearer-token auth (off by default); transport mode (stdio / HTTP) selectable
  in the UI

  **远程 MCP HTTP 服务** —— 仅限本地回环（`127.0.0.1`），可选 Bearer Token 鉴权（默认关闭）；stdio / HTTP 传输模式可在 UI 中选择。

- **AI vision for multimodal models** — `get_world_view` renders a top-down PNG
  of nearby blocks and returns it as base64

  **多模态模型的 AI 视觉** —— `get_world_view` 可渲染附近方块的俯视 PNG 图并以 base64 返回。

- **Smart movement & flight** — `smart_move` auto-jumps over 1-block gaps and
  stops on larger obstacles; `fly_to` flies to a target in creative mode.
  Moving tools (`move_to` / `walk_direction` / `smart_move` / `fly_to` /
  `collect_items` / `attack_entity`) report the bot's end `position` in their
  result

  **智能移动与飞行** —— `smart_move` 可自动跳过 1 格宽的缺口，遇到更大障碍会停下；`fly_to` 可在创造模式下飞向目标。会移动机器人的工具（`move_to` / `walk_direction` / `smart_move` / `fly_to` / `collect_items` / `attack_entity`）都会在结果中报告结束 `position`。

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

- **Command feedback verification** — `execute_command` reads the server's
  chat reply after sending: every System message in the feedback window is
  scanned, so a rejected command (e.g. `Incorrect argument for command ...`,
  `Unknown item '...'` from an invalid `/give` id, or Minecraft's two-line
  "Unknown or incomplete command …<--[HERE]" pair) returns a
  `CommandRejected` error (-32009) with the server's verbatim feedback
  instead of a fake "Executed command" success. `drop_item` verifies the
  inventory actually changed after the click.

  **命令反馈校验** —— `execute_command` 在发送后会读取服务器的聊天回复：反馈窗口内的**每一条** System 消息都会被扫描，因此被拒绝的命令（如 `Incorrect argument for command ...`、`/give` 无效物品 ID 的 `Unknown item '...'`，或 Minecraft 的"Unknown or incomplete command …<--[HERE]" 两行式拒绝）会返回 `CommandRejected` 错误（-32009）并附上服务器的原始反馈，而不是伪造的"已执行"。`drop_item` 会在点击后校验背包确实发生了变化。

- **Fresh state on demand** — `get_self_info` / `get_inventory` accept
  `force=true` (default) to trigger an immediate snapshot rebuild before
  reading, so an agent that just dropped an item / moved / teleported sees the
  fresh state instead of a 500 ms-stale snapshot. `get_server_info` probes
  `commands_enabled` live via `/seed` and caches the result.

  **按需获取最新状态** —— `get_self_info` / `get_inventory` 接受 `force=true`（默认）参数，读取前会立即触发快照重建，因此刚丢弃物品 / 移动 / 传送后的代理能读到最新状态，而非 500ms 前的旧快照。`get_server_info` 通过 `/seed` 实时探测 `commands_enabled` 并缓存结果。

## Tool Categories

| Category / 类别 | Tools / 工具 |
|-----------------|--------------|
| **Query / 查询** | `get_self_info`, `get_inventory`, `get_hotbar`, `get_bot_status`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **Movement / 移动** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **Block / 方块** | `break_block`, `place_block`, `use_item_on_block` |
| **Item / 物品** | `drop_item`, `set_hotbar_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items`, `give_item` |
| **Container / 容器** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **Combat / 战斗** | `attack_entity`, `shield_block` |
| **Chat / 聊天** | `send_chat`, `execute_command`, `set_game_mode` |
| **Settings / 设置** | `get_settings`, `update_settings`, `connect_bot`, `disconnect_bot` |
| **Unified / 统一** | `act` — one tool that can move, smart-move, fly, mine, attack, or collect items and returns an environment snapshot (`perception_radius` 0-32 trims the returned nearby blocks/entities payload) |

上述工具按功能领域分类；`act` 是一个统一入口，能够根据传入参数执行移动、智能移动、飞行、挖掘、攻击或拾取物品，并返回环境快照。设置类工具（`get_settings` / `update_settings` / `connect_bot` / `disconnect_bot`）在机器人离线时也可用——LLM 客户端可以自行读取和修改所有配置，包括切换 Minecraft 服务器地址。

## Known limitations / 已知限制

- **Fluid buckets cannot be placed via `use_item_on_block`** — azalea 0.15.1's
  block interaction fabricates the hit result (block centre, fixed Up face),
  which the vanilla server rejects for bucket `UseItemOn` while accepting
  block placements and flint-and-steel on the same path. The tool returns
  `success:false` with `reason: "bucket_placement_unsupported"` and suggests
  the working `execute_command` alternatives (`/setblock <x> <y> <z> water` /
  `/fill`). There is no upstream azalea API to send a real raycast hit yet.

  **流体桶无法通过 `use_item_on_block` 放置** —— azalea 0.15.1 的方块交互伪造了命中结果（方块中心、固定 Up 面），原版服务器在桶的 `UseItemOn` 场景会拒绝该命中，而方块放置和打火石走同一路径却能成功。该工具会返回 `success:false` 且携带 `reason: "bucket_placement_unsupported"`，并提示可用的 `execute_command` 替代方案（`/setblock <x> <y> <z> water` / `/fill`）。上游 azalea 目前没有提供发送真实射线命中结果的 API。

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

## Contributing / 参与贡献

See `CONTRIBUTING.md` for the branch model and release pipeline: `develop`
(integration) → `release` (auto pre-release, npm `next`) → `master`
(stable tag `vX.Y.Z`, npm `latest`). All changes go through PRs with user
review — never commit to `master` or `release` directly.

详见 `CONTRIBUTING.md`：分支模型为 `develop`（集成）→ `release`（自动预发布，npm
`next`）→ `master`（稳定 tag `vX.Y.Z`，npm `latest`）。所有改动走 PR 并经用户审阅，
禁止直接提交 `master` / `release`。

## Language / 语言切换

The desktop UI supports **English** and **简体中文**. Pick a language from the
**Language** dropdown in the Settings panel — the change takes effect on the
next frame without reconnecting. On startup the app auto-detects the system's
default CJK font (Windows `msyh.ttc`, macOS `PingFang.ttc`, Linux Noto /
WenQuanYi) so Chinese text renders correctly without manual font setup.

桌面 UI 支持 **英文** 与 **简体中文**。在"设置"面板的"语言"下拉框中切换，下一帧即
生效，无需重连。启动时会自动探测系统默认中文字体（Windows `msyh.ttc`、macOS
`PingFang.ttc`、Linux Noto / 文泉驿），无需手动安装字体即可正常显示中文。

## Install via npm / 通过 npm 安装

No Rust toolchain needed — prebuilt binaries are published to npm for Windows
x64/arm64, macOS arm64, and Linux x64/arm64. Install globally:

无需 Rust 工具链——预编译二进制已发布到 npm，支持 Windows x64/arm64、macOS arm64 和 Linux x64/arm64。全局安装：

```bash
npm install -g minecraft-mcp-rs
```

or run it directly without installing (each platform's binary is downloaded on
demand):

或无需安装直接运行（各平台的二进制按需下载）：

```bash
npx -y minecraft-mcp-rs@1.3.1 --headless --stdio
bunx minecraft-mcp-rs@1.3.1 --headless --stdio
```

Ready-to-paste Claude Desktop / Cursor config (stdio implies the bot runs
headless and exits when the MCP client disconnects):

可直接粘贴到 Claude Desktop / Cursor 的配置（stdio 意味着机器人以无头模式运行，并在 MCP 客户端断开时退出进程）：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.3.1", "--headless", "--stdio"]
    }
  }
}
```

For bunx users, replace `"command": "npx"` with `"command": "bunx"` and drop
the `-y` flag.

使用 bunx 的用户请将 `"command": "npx"` 替换为 `"command": "bunx"`，并去掉 `-y` 参数。

> For maintainers: npm publishing authenticates via npm **Trusted Publishing
> (OIDC)** — each package on npmjs.com → Settings → Trusted Publishers →
> GitHub Actions, owner `halfoffive`, workflow `release.yml`. When OIDC is
> not configured, the `NPM_TOKEN` secret (a granular access token) is used
> instead. NEVER commit the token or any fragment of it anywhere in the
> repository.

> 维护者须知：npm 发布通过 npm **Trusted Publishing（OIDC）** 认证 —— 在 npmjs.com 上每个包的 Settings → Trusted Publishers → GitHub Actions，属主 `halfoffive`，workflow `release.yml`。若未配置 OIDC，则回退使用 `NPM_TOKEN` 密钥（细粒度访问令牌）。切勿将令牌或其任何片段提交到仓库的任何位置。

### CLI flags / 命令行参数

The binary accepts a small set of flags (run `minecraft-mcp-rs --help` for the
full usage):

二进制接受少量命令行参数（运行 `minecraft-mcp-rs --help` 查看完整用法）：

| Flag / 参数 | Description / 说明 |
|-------------|---------------------|
| `--headless` | Run without the desktop window; auto-connect the bot and exit when the MCP transport closes (Ctrl+C, client pipe break, or after 10 minutes with no bot command) / 无桌面窗口运行；自动连接机器人，MCP 传输关闭（Ctrl+C、客户端管道断开，或 10 分钟无任何机器人命令）时退出进程 |
| `--gui` | Open the desktop window explicitly / 显式打开桌面窗口 |
| `--stdio` | Force the MCP stdio transport; implies headless when used alone (overrides the configured transport) / 强制使用 MCP stdio 传输；单独使用时隐含无头模式（覆盖配置中的传输方式） |
| `-h`, `--help` | Print usage to stderr and exit / 打印用法到 stderr 并退出 |

With NO arguments the binary prints help and exits; use `--gui` to open the
desktop UI.

不带任何参数时，二进制打印帮助信息并退出；使用 `--gui` 打开桌面 UI。

## Quick Start

### Prerequisites

- [Rust nightly](https://rustup.rs/) (pinned in `rust-toolchain.toml`, edition 2024; azalea 0.15.1 requires nightly)
- A **Minecraft Java Edition 1.21.11** server (local or remote) — **the only supported Minecraft version**
- An MCP client (Claude Desktop, Cursor, or any MCP-compatible LLM host)

前置条件：需要安装 [Rust nightly](https://rustup.rs/)（`rust-toolchain.toml` 已固定，edition 2024；azalea 0.15.1 要求 nightly）、一个 **Minecraft Java Edition 1.21.11** 服务器（本地或远程，**唯一受支持的 Minecraft 版本**），以及一个 MCP 客户端（Claude Desktop、Cursor 或任何兼容 MCP 的 LLM 宿主）。

### Build

```bash
cargo build
```

构建项目。

### Run

```bash
cargo run              # no args: prints help and exits
cargo run -- --gui     # starts the egui desktop UI
cargo run -- --stdio   # headless: stdio MCP server only, no window
```

`cargo run` with no arguments prints the help text and exits. Pass `--gui` to
start the egui desktop UI, or `--stdio` alone to run headless (the MCP server
listens on stdin/stdout, the mode Claude Desktop / Cursor use). In the UI,
choose the MCP transport in the Settings panel:

- **stdio** — the MCP server listens on stdin/stdout (default for Claude
  Desktop / Cursor).
- **HTTP** — the MCP server binds to `127.0.0.1` only; set the port and
  Bearer token (a random UUID v4 is generated on each `AppConfig::default()`;
  override it in the Settings panel). The MCP Config panel generates the
  matching JSON config for copying into your MCP client.

运行 `cargo run`（不带参数）会打印帮助信息并退出。传入 `--gui` 启动 egui 桌面 UI，单独传入 `--stdio` 则以无头模式运行（MCP 服务器监听标准输入 / 输出，即 Claude Desktop / Cursor 使用的模式）。在 UI 的“设置”面板中选择 MCP 传输方式：

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

## Continuous Integration

CI/CD runs exclusively on **GitHub Actions** (`.github/workflows/`) —
`build.yml` (dev binary matrix + lint), `release.yml` (two-channel release:
push to `release` → auto pre-release with npm `next`; tag `vX.Y.Z` on
`master` → stable release with npm `latest`), and `deploy-docs.yml`
(VitePress site build + Pages deployment).

CI/CD 全部运行在 **GitHub Actions**（`.github/workflows/`）——`build.yml`
（dev 二进制矩阵构建 + lint）、`release.yml`（双通道发布：push 到 `release` →
自动预发布 + npm `next`；在 `master` 打 `vX.Y.Z` tag → 稳定发布 + npm
`latest`）、`deploy-docs.yml`（VitePress 站点构建 + Pages 部署）。

## Configuration

All settings have sensible defaults and can be changed at runtime through the
egui settings panel (fully editable — text inputs for strings, DragValue
sliders for numeric fields). After editing, click **Connect** to apply the
settings and spawn the bot connection on a dedicated background thread.
Alternatively, an LLM agent can read and change every setting through the MCP
settings tools (`get_settings` / `update_settings`).

所有设置均有合理默认值，可在运行时通过 egui 设置面板修改（完全可编辑——字符串使用文本输入框，数值字段使用 DragValue 滑块）。编辑完成后点击 **Connect**，设置即会生效，并在专用后台线程上启动机器人连接。AI 代理也可以通过 MCP 设置工具（`get_settings` / `update_settings`）读取和修改所有设置。

| Field / 字段 | Default / 默认值 | Description / 说明 |
|--------------|------------------|--------------------|
| `mc_address` | `127.0.0.1` | Minecraft server address / Minecraft 服务器地址 |
| `mc_port` | `25565` | Minecraft server port / Minecraft 服务器端口 |
| `ai_username` | `AI_Bot` | Bot in-game username / 机器人游戏内用户名 |
| `mcp_transport` | `Http` | MCP transport: `Stdio` or `Http` / MCP 传输方式：`Stdio` 或 `Http` |
| `mcp_address` | `127.0.0.1` | MCP HTTP bind address. Non-loopback binds require `mcp_auth_enabled=true` (validation rejects an unauthenticated non-loopback HTTP bind) / MCP HTTP 绑定地址。绑定非回环地址时必须启用 `mcp_auth_enabled=true`（校验会拒绝无鉴权的非回环 HTTP 绑定） |
| `mcp_port` | `3000` | MCP HTTP port / MCP HTTP 端口 |
| `mcp_token` | random UUID v4 | Bearer token for HTTP transport, used only when auth is enabled (generated on each `AppConfig::default()`) / HTTP 传输的 Bearer Token，仅在启用鉴权时使用（每次 `AppConfig::default()` 时随机生成） |
| `mcp_auth_enabled` | `false` | Require a Bearer token over HTTP / 要求 HTTP 请求携带 Bearer Token |
| `language` | `En` | UI language: `En` or `ZhCn` / UI 语言：`En` 或 `ZhCn` |
| `chunk_scan_radius` | `8` | Chunks to scan (1–16) / 扫描区块半径（1–16） |
| `block_perception_radius` | `32` | Block awareness range (8–64) / 方块感知范围（8–64） |
| `snapshot_interval_ms` | `500` | World snapshot interval / 世界快照间隔（毫秒） |
| `reconnect_initial_delay_ms` | `5000` | Initial reconnect backoff / 初始重连退避（毫秒） |
| `reconnect_max_delay_ms` | `60000` | Maximum reconnect backoff / 最大重连退避（毫秒） |
| `command_timeout_secs` | `30` | Bot command timeout / 机器人命令超时（秒） |
| `fly_timeout_secs` | `60` | Timeout for long `fly_to` flights / `fly_to` 长距离飞行超时（秒） |

数值型字段均可在 UI 中通过滑块或键盘输入调整；修改后需点击 **Connect** 才会应用到机器人连接。

### Configuration via environment variables / 环境变量配置

Configuration is read **exclusively from environment variables** (12-factor
style, like cargo) — there is **no config file** anymore. Each setting starts
from its default and is overridden when the corresponding `MINECRAFT_MCP_*`
variable is present:

配置**完全从环境变量读取**（12-factor 风格，同 cargo）——**不再有配置文件**。每项设置先取默认值，再被对应的 `MINECRAFT_MCP_*` 环境变量覆盖：

| Variable / 环境变量 | Field / 字段 | Default / 默认 |
|--------------------|--------------|----------------|
| `MINECRAFT_MCP_MC_ADDRESS` | `mc_address` | `127.0.0.1` |
| `MINECRAFT_MCP_MC_PORT` | `mc_port` | `25565` |
| `MINECRAFT_MCP_AI_USERNAME` | `ai_username` | `AI_Bot` |
| `MINECRAFT_MCP_MCP_ADDRESS` | `mcp_address` | `127.0.0.1` |
| `MINECRAFT_MCP_MCP_PORT` | `mcp_port` | `3000` |
| `MINECRAFT_MCP_TASK_NAME` | `task_name` | `mining` |
| `MINECRAFT_MCP_CHUNK_SCAN_RADIUS` | `chunk_scan_radius` | `8` |
| `MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS` | `block_perception_radius` | `32` |
| `MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS` | `snapshot_interval_ms` | `500` |
| `MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS` | `reconnect_initial_delay_ms` | `5000` |
| `MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS` | `reconnect_max_delay_ms` | `60000` |
| `MINECRAFT_MCP_COMMAND_TIMEOUT_SECS` | `command_timeout_secs` | `30` |
| `MINECRAFT_MCP_FLY_TIMEOUT_SECS` | `fly_timeout_secs` | `60` |
| `MINECRAFT_MCP_TOKEN` | `mcp_token` | random UUID v4 / 随机 UUID v4 |
| `MINECRAFT_MCP_AUTH_ENABLED` | `mcp_auth_enabled` | `false` |
| `MINECRAFT_MCP_TRANSPORT` | `mcp_transport` | `http` |
| `MINECRAFT_MCP_LANGUAGE` | `language` | system locale / 系统语言 |

> **Security note:** plain HTTP + a non-loopback `mcp_address` with
> `mcp_auth_enabled=false` is rejected at startup (`AppConfig::validate`).
> To expose the MCP server beyond localhost, enable Bearer auth.
>
> **安全提示:** `mcp_address` 为非回环地址、HTTP 传输且未启用鉴权的组合会在启动时被拒绝。如需将 MCP 服务暴露到本机之外，请启用 Bearer Token 鉴权。

Malformed variable values log a warning and keep the default — startup never
fails because of an environment typo. Semantically invalid values that parse
but would wedge the runtime (`0` for ports/durations, out-of-range radii) are
also rejected per-field with a warning and the default is kept, followed by a
final full-config validation gate. `MINECRAFT_MCP_TOKEN` is the ONLY way to
pin the MCP bearer token; without it a fresh random UUID is generated per
process start.

非法环境变量值只会记录警告并保留默认值——不会因环境变量拼写错误而启动失败。能解析但语义非法的值（端口/时长/半径等为 `0` 或越界）也会按字段警告并回退默认值，随后再做一次全配置校验兜底。`MINECRAFT_MCP_TOKEN` 是固定 MCP Bearer Token 的唯一途径；未设置时每次启动都会生成新的随机 UUID。

Runtime changes (UI settings panel, `update_settings` MCP tool) apply to the
running process only — restart with the environment variables to persist them.
Changing `mc_address`/`mc_port`/`ai_username` while connected triggers an
automatic reconnect, while `mcp_transport`/`mcp_address`/`mcp_port` take
effect on the next process restart.

运行时修改（UI 设置面板、`update_settings` MCP 工具）仅对当前进程生效——如需持久化，请在重启时通过环境变量配置。已连接时修改 `mc_address`/`mc_port`/`ai_username` 会自动触发重连，而 `mcp_transport`/`mcp_address`/`mcp_port` 的变更在下次进程重启时生效。

### Error contract / 错误契约

Every MCP error carries a JSON-RPC code plus structured `data` with a
machine-readable `reason`, a `retryable` bool, and variant-specific fields, so
AI agents can distinguish "bot is gone, retry later" from "input is invalid":

每个 MCP 错误都携带 JSON-RPC code 以及结构化 `data`（含机器可读的 `reason`、`retryable` 布尔值和各变体专属字段），AI 代理可以据此区分"机器人已断开，稍后重试"与"输入无效"：

| Code / 错误码 | Variant / 变体 | `reason` | retryable |
|---------------|----------------|----------|-----------|
| -32000 | `Offline` | `bot_disconnected` | true |
| -32001 | `CommandTimeout` | `command_timeout` | true |
| -32002 | `BlockNotFound` | `block_not_found` | false |
| -32003 | `ChunkNotLoaded` | `chunk_not_loaded` | true |
| -32004 | `InventoryFull` | `inventory_full` | false |
| -32005 | `MiningInterrupted` | `mining_interrupted` | false |
| -32006 | `ContainerAlreadyOpen` | `container_already_open` | false |
| -32007 | `ContainerTimeout` | `container_timeout` | true |
| -32008 | `PathfindingFailed` | `pathfinding_failed` | false |
| -32009 | `CommandRejected` | `command_rejected` | true |
| -32010 | `ContainerNotOpen` | `container_not_open` | false |
| -32600 | `PermissionDenied` | `permission_denied` | false |
| -32602 | `ToolNotFound` / `TooFar` / `InvalidParams` | `tool_not_found` / `too_far` / `invalid_params` | false |
| -32603 | `Internal` | `internal_error` | false |

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
