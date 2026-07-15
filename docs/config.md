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

## Logging

All log output goes to **stderr** only — stdout is reserved for MCP JSON-RPC
transport. Default filter: `minecraft_mcp_rs=debug, azalea=warn`.
