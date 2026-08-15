# Configuration

All settings have sensible defaults. Configuration is read **exclusively
from environment variables** (12-factor style, like cargo) — there is **no
config file** anymore (the old `config.json` / `--config <path>` mechanism
was removed).

Each setting starts from its default and is overridden when the
corresponding `MINECRAFT_MCP_*` variable is present. Malformed values log a
warning and keep the default — startup never fails because of an
environment typo.

| Variable | Field | Default | Description |
|----------|-------|---------|-------------|
| `MINECRAFT_MCP_MC_ADDRESS` | `mc_address` | `127.0.0.1` | Minecraft server address |
| `MINECRAFT_MCP_MC_PORT` | `mc_port` | `25565` | Minecraft server port |
| `MINECRAFT_MCP_AI_USERNAME` | `ai_username` | `AI_Bot` | Bot in-game username |
| `MINECRAFT_MCP_MCP_ADDRESS` | `mcp_address` | `127.0.0.1` | MCP HTTP bind address |
| `MINECRAFT_MCP_MCP_PORT` | `mcp_port` | `3000` | MCP HTTP port |
| `MINECRAFT_MCP_TASK_NAME` | `task_name` | `mining` | Descriptive task name for the UI |
| `MINECRAFT_MCP_CHUNK_SCAN_RADIUS` | `chunk_scan_radius` | `8` | Chunks to scan (1–16) |
| `MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS` | `block_perception_radius` | `32` | Block awareness range (8–64) |
| `MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS` | `snapshot_interval_ms` | `500` | World snapshot interval (ms); idle bots relax to at least 5000 ms |
| `MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS` | `reconnect_initial_delay_ms` | `5000` | Initial reconnect backoff (ms) |
| `MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS` | `reconnect_max_delay_ms` | `60000` | Maximum reconnect backoff (ms) |
| `MINECRAFT_MCP_COMMAND_TIMEOUT_SECS` | `command_timeout_secs` | `30` | Bot command timeout (s) |
| `MINECRAFT_MCP_FLY_TIMEOUT_SECS` | `fly_timeout_secs` | `60` | Timeout for long `fly_to` flights (s) |
| `MINECRAFT_MCP_TOKEN` | `mcp_token` | random UUID v4 | Bearer token for HTTP transport; the ONLY way to pin it |
| `MINECRAFT_MCP_AUTH_ENABLED` | `mcp_auth_enabled` | `false` | Require a Bearer token over HTTP (off by default) |
| `MINECRAFT_MCP_TRANSPORT` | `mcp_transport` | `http` | MCP transport: `stdio` or `http` |
| `MINECRAFT_MCP_LANGUAGE` | `language` | system locale | UI language: `en` or `zh_cn` |

Example:

```bash
export MINECRAFT_MCP_MC_ADDRESS=play.example.com
export MINECRAFT_MCP_MC_PORT=25565
export MINECRAFT_MCP_TRANSPORT=stdio
minecraft-mcp-rs --headless
```

> Windows PowerShell: `$env:MINECRAFT_MCP_MC_ADDRESS="play.example.com"`

## Runtime changes

The egui settings panel and the `update_settings` MCP tool change the
running process only — nothing is written to disk. Restart with the
environment variables to persist. Changing `mc_address`/`mc_port`/
`ai_username` while connected triggers an automatic reconnect;
`mcp_transport`/`mcp_address`/`mcp_port` take effect on the next process
restart.

## Headless mode & CLI flags

The binary accepts a small set of flags (parsed by clap, help generated from
the flag docs and printed to stderr):

| Flag | Description |
|------|-------------|
| `--gui` | Open the desktop UI |
| `--headless` | No desktop window; auto-connect the bot and exit when the MCP transport closes |
| `--stdio` | Force the MCP stdio transport (overrides the env config); alone (without `--gui`) this implies headless mode |
| `-h`, `--help` | Print usage to stderr and exit 0 |
| `-V`, `--version` | Print the version to stderr and exit 0 |

Precedence: `--headless` wins over `--gui`; `--stdio` alone implies
headless; anything else runs the GUI. With no arguments the binary prints
help and exits 0 (it never starts the MCP server in that mode); a usage
error prints to stderr and exits 2.

In headless mode the bot auto-connects on startup, and an agent-driven
config change (via the `update_settings` MCP tool) that touches the server
connection fields reconnects automatically. `mcp_transport`/`mcp_address`/
`mcp_port` changes take effect on the next process restart.
