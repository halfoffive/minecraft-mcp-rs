# Tools

`minecraft-mcp-rs` exposes **41 MCP tools** organized into 8 domains, plus a
unified `act` tool. Each tool module (`mcp/tools_*.rs`) exposes a builder
function, and tool parameters are annotated with
`#[derive(schemars::JsonSchema)]` so clients get accurate JSON schemas.

## Tool Categories

| Category | Tools |
|----------|-------|
| **Query** | `get_self_info`, `get_inventory`, `get_hotbar`, `get_bot_status`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **Movement** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **Block** | `break_block`, `place_block`, `use_item_on_block` |
| **Item** | `drop_item`, `set_hotbar_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items`, `give_item` |
| **Container** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **Combat** | `attack_entity`, `shield_block` |
| **Chat** | `send_chat`, `execute_command`, `set_game_mode` |
| **Settings** | `get_settings`, `update_settings`, `connect_bot`, `disconnect_bot` |
| **Unified** | `act` — one tool that can move, smart-move, fly, mine, attack, or collect items and returns an environment snapshot. `perception_radius` (0-32, default = configured `block_perception_radius`) bounds the nearby blocks/entities payload — the default radius-32 result is >1 MB, so pass a small radius for iterative loops |

## Hotbar, status & give helpers

- `get_hotbar` — the 9 hotbar slots (0-8) plus `held_item_slot`; empty
  slots render as `null`. The single explicit view of the slot layout that
  `set_hotbar_item` / `equip_tool` / `drop_item` operate on.
- `get_bot_status` — cheap polling endpoint for long-running operations
  (`fly_to`, mining, `collect_items`): `connected`, `bot_busy`,
  block + precise position, `yaw`, vitals, and snapshot age. Reads the
  cached snapshot by default and reports `connected:false` while offline.
- `give_item` — the smoke-test command fallback packaged as a tool: runs
  `/give <bot> <item> <count>`, then for `target=hotbar` also
  `/item replace entity <bot> hotbar.<slot> with <item> <count>` (falling
  back to the swap-click `set_hotbar_item` move when the server rejects
  `/item replace`). Requires server commands (OP).

## Settings & lifecycle tools

These four tools work even while the bot is **offline** — a client must be
able to change the Minecraft server address before the first connect:

| Tool | Description |
|------|-------------|
| `get_settings` | Returns the full configuration (the MCP token is redacted to `"***"`) plus runtime status: `online`, `connecting`, `mcp_server_status`. |
| `update_settings` | Partial update — only provided fields change. Validated **before** being applied to the running process (there is no config file; restart with `MINECRAFT_MCP_*` environment variables to persist). Changing `mc_address`/`mc_port`/`ai_username` triggers a reconnect when connected/connecting; `mcp_transport`/`mcp_address`/`mcp_port` take effect on process restart. |
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
| -32009 | `CommandRejected` | `command_rejected` | true |
| -32010 | `ContainerNotOpen` | `container_not_open` | false |
| -32600 | `PermissionDenied` | `permission_denied` | false |
| -32602 | `ToolNotFound` / `TooFar` / `InvalidParams` | `tool_not_found` / `too_far` / `invalid_params` | false |
| -32603 | `Internal` | `internal_error` | false |

## Notes

- **Honest error reporting** — `BotError::InvalidParams` maps to MCP
  `INVALID_PARAMS`; unbreakable blocks return `MiningInterrupted` instead of
  panicking; `set_game_mode` flags the OP requirement.
- **Command feedback verification** — `execute_command` reads the server's
  chat reply after sending: a rejected command returns a `CommandRejected`
  error (-32009) with the server's verbatim feedback, and successful commands
  attach the server's reply (e.g. "Teleported X to ...") to the result.
  `drop_item` verifies the inventory slot actually changed after the click
  and reports `success: false` when it did not.
- **Fresh state on demand** — `get_self_info` / `get_inventory` accept
  `force=true` (default) and trigger an immediate snapshot rebuild before
  reading. `get_server_info` probes `commands_enabled` live via `/seed`
  (cached until `refresh=true`) and reports `bot_busy`.
- **`set_hotbar_item`** — moves an existing inventory stack into a hotbar
  slot (0-8) via a container swap-click, a reliable alternative to `/item
  replace` whose syntax varies across servers. The item must already be in
  the inventory; it cannot conjure items.
- **AI vision** — `get_world_view` renders a top-down PNG of nearby blocks
  (`mcp/render.rs`) and returns it as base64 for multimodal models.
- **Compound operations** — higher-level state machines (e.g. mine-and-collect)
  are built on primitive commands in `compound_ops.rs`.
