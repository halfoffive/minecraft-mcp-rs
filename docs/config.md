# Configuration

All settings have sensible defaults and can be changed at runtime through the
egui settings panel — fully editable, with text inputs for strings and
`DragValue` sliders for numeric fields. After editing, click **Connect** to
apply the settings and spawn the bot connection on a dedicated background
thread.

| Field | Default | Description |
|-------|---------|-------------|
| `mc_address` | `127.0.0.1` | Minecraft server address |
| `mc_port` | `25565` | Minecraft server port |
| `ai_username` | `AI_Bot` | Bot in-game username |
| `mcp_transport` | `Http` | MCP transport: `Stdio` or `Http` |
| `mcp_address` | `127.0.0.1` | MCP HTTP bind address (loopback only) |
| `mcp_port` | `3000` | MCP HTTP port |
| `mcp_token` | `minecraft-mcp-rs` | Bearer token for HTTP transport |
| `chunk_scan_radius` | `8` | Chunks to scan (1–16) |
| `block_perception_radius` | `32` | Block awareness range (8–64) |
| `snapshot_interval_ms` | `500` | World snapshot interval |
| `reconnect_initial_delay_ms` | `5000` | Initial reconnect backoff |
| `reconnect_max_delay_ms` | `60000` | Maximum reconnect backoff |
| `command_timeout_secs` | `30` | Bot command timeout |

## Config file persistence

Settings are persisted to `config.json` in the OS config directory and
reloaded on every startup — file values override defaults:

| OS | Path |
|----|------|
| Windows | `%APPDATA%\minecraft-mcp-rs\config.json` |
| Linux | `~/.config/minecraft-mcp-rs/config.json` |
| macOS | `~/Library/Application Support/minecraft-mcp-rs/config.json` |

The write is atomic (temp file + rename, `0600` on Unix). Point the binary at
a different file with `--config <path>`.

## Headless mode & CLI flags

The binary accepts a small set of flags:

| Flag | Description |
|------|-------------|
| `--headless` | No desktop window; auto-connect the bot and exit when the MCP transport closes |
| `--stdio` | Force the MCP stdio transport (overrides the config) |
| `--config <path>` | Load the config file at `<path>` |
| `-h`, `--help` | Print usage to stderr and exit |

In headless mode the bot auto-connects on startup, and an agent-driven config
change (via the `update_settings` MCP tool) that touches the server
connection fields reconnects automatically. `mcp_transport`/`mcp_address`/
`mcp_port` changes take effect on the next process restart.

## Logging

All log output goes to **stderr** only — stdout is reserved for MCP JSON-RPC
transport. Default filter: `minecraft_mcp_rs=debug, azalea=warn`.
