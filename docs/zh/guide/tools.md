# 工具

`minecraft-mcp-rs` 暴露了 **30+ MCP 工具**，组织为 7 个领域，外加一个统一的
`act` 工具。每个工具模块（`mcp/tools_*.rs`）暴露一个构建函数，工具参数使用
`#[derive(schemars::JsonSchema)]` 标注，以便客户端获得准确的 JSON schema。

## 工具分类

| 类别 | 工具 |
|----------|-------|
| **查询（Query）** | `get_self_info`, `get_inventory`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **移动（Movement）** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **方块（Block）** | `break_block`, `place_block`, `use_item_on_block` |
| **物品（Item）** | `drop_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items` |
| **容器（Container）** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **战斗（Combat）** | `attack_entity`, `shield_block` |
| **聊天（Chat）** | `send_chat`, `execute_command`, `set_gamemode` |
| **统一（Unified）** | `act` —— 一个可以移动、智能移动、飞行、挖掘、攻击或收集物品，并返回环境快照的工具 |

## 说明

- **诚实的错误报告** —— `BotError::InvalidParams` 映射到 MCP
  `INVALID_PARAMS`；不可破坏的方块返回 `MiningInterrupted` 而非 panic；
  `set_game_mode` 会提示需要 OP 权限。
- **AI 视觉** —— `get_world_view` 渲染附近方块的俯视 PNG 图
  （`mcp/render.rs`）并以 base64 返回，供多模态模型使用。
- **复合操作** —— 在 `compound_ops.rs` 中基于基本命令构建更高层的状态机
  （例如挖取并收集）。
