# Tools

`minecraft-mcp-rs` exposes **30+ MCP tools** organized into 7 domains, plus a
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
| **Chat** | `send_chat`, `execute_command`, `set_gamemode` |
| **Unified** | `act` — one tool that can move, smart-move, fly, mine, attack, or collect items and returns an environment snapshot |

## Notes

- **Honest error reporting** — `BotError::InvalidParams` maps to MCP
  `INVALID_PARAMS`; unbreakable blocks return `MiningInterrupted` instead of
  panicking; `set_game_mode` flags the OP requirement.
- **AI vision** — `get_world_view` renders a top-down PNG of nearby blocks
  (`mcp/render.rs`) and returns it as base64 for multimodal models.
- **Compound operations** — higher-level state machines (e.g. mine-and-collect)
  are built on primitive commands in `compound_ops.rs`.
