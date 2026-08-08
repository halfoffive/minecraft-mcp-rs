# Tools

`minecraft-mcp-rs` exposes **30+ MCP tools** organized into 8 domains, plus a
unified `act` tool. Each tool module (`mcp/tools_*.rs`) exposes a builder
function, and tool parameters are annotated with
`#[derive(schemars::JsonSchema)]` so clients get accurate JSON schemas.

## Tool Categories

| Category | Tools |
|----------|-------|
| **Query** | `get_self_info`, `get_inventory`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **Movement** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **Block** | `break_block`, `place_block`, `use_item_on_block` |
| **Item** | `drop_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items` |
| **Container** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **Combat** | `attack_entity`, `shield_block` |
| **Chat** | `send_chat`, `execute_command`, `set_game_mode` |
| **Settings** | `get_settings`, `update_settings`, `connect_bot`, `disconnect_bot` |
| **Unified** | `act` — one tool that can move, smart-move, fly, mine, attack, or collect items and returns an environment snapshot |

## Settings & lifecycle tools

These four tools work even while the bot is **offline** — a client must be
able to change the Minecraft server address before the first connect:

| Tool | Description |
|------|-------------|
| `get_settings` | Returns the full configuration (the MCP token is redacted to `"***"`) plus runtime status: `online`, `connecting`, `mcp_server_status`, `config_path`. |
| `update_settings` | Partial update — only provided fields change. Validated, persisted to the config file **before** being applied. Changing `mc_address`/`mc_port`/`ai_username` triggers a reconnect when connected/connecting; `mcp_transport`/`mcp_address`/`mcp_port` take effect on process restart. |
| `connect_bot` | Starts the bot connection to the configured server. No-op if already connected or connecting. |
| `disconnect_bot` | Requests a disconnect; the reconnect loop stops. |

## Error contract

Every MCP error carries a JSON-RPC code plus structured `data` with a
machine-readable `reason`, a `retryable` bool, and variant-specific fields:

| Code | Variant | `reason` | retryable |
|------|---------|----------|-----------|
| -32000 | `Offline` | `bot_disconnected` | true |
| -32001 | `CommandTimeout` | `command_timeout` | true |
| -32002 | `BlockNotFound` | `block_not_found` | false |
| -32003 | `ChunkNotLoaded` | `chunk_not_loaded` | true |
| -32004 | `InventoryFull` | `inventory_full` | false |
| -32005 | `MiningInterrupted` | `mining_interrupted` | false |
| -32006 | `ContainerAlreadyOpen` | `container_already_open` | false |
| -32007 | `ContainerTimeout` | `container_timeout` | true |
| -32008 | `PathfindingFailed` | `pathfinding_failed` | false |
| -32600 | `PermissionDenied` | `permission_denied` | false |
| -32602 | `ToolNotFound` / `TooFar` / `InvalidParams` | `tool_not_found` / `too_far` / `invalid_params` | false |
| -32603 | `Internal` | `internal_error` | false |

## Notes

- **Honest error reporting** — `BotError::InvalidParams` maps to MCP
  `INVALID_PARAMS`; unbreakable blocks return `MiningInterrupted` instead of
  panicking; `set_game_mode` flags the OP requirement.
- **AI vision** — `get_world_view` renders a top-down PNG of nearby blocks
  (`mcp/render.rs`) and returns it as base64 for multimodal models.
- **Compound operations** — higher-level state machines (e.g. mine-and-collect)
  are built on primitive commands in `compound_ops.rs`.
