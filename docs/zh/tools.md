# 工具

`minecraft-mcp-rs` 暴露了 **41 个 MCP 工具**，组织为 8 个领域，外加一个统一的
`act` 工具。每个工具模块（`mcp/tools_*.rs`）暴露一个构建函数，工具参数使用
`#[derive(schemars::JsonSchema)]` 标注，以便客户端获得准确的 JSON schema。

## 工具分类

| 类别 | 工具 |
|----------|-------|
| **查询（Query）** | `get_self_info`, `get_inventory`, `get_hotbar`, `get_bot_status`, `get_nearby_blocks`, `get_nearby_entities`, `get_chunk_summary`, `is_connected`, `get_chat_history`, `get_server_info`, `get_world_view` |
| **移动（Movement）** | `move_to`, `walk_direction`, `jump`, `teleport`, `smart_move`, `fly_to` |
| **方块（Block）** | `break_block`, `place_block`, `use_item_on_block` |
| **物品（Item）** | `drop_item`, `set_hotbar_item`, `equip_tool`, `switch_hotbar_slot`, `use_item`, `collect_items`, `give_item` |
| **容器（Container）** | `open_container`, `take_from_container`, `put_into_container`, `close_container` |
| **战斗（Combat）** | `attack_entity`, `shield_block` |
| **聊天（Chat）** | `send_chat`, `execute_command`, `set_game_mode` |
| **设置（Settings）** | `get_settings`, `update_settings`, `connect_bot`, `disconnect_bot` |
| **统一（Unified）** | `act` —— 一个可以移动、智能移动、飞行、挖掘、攻击或收集物品，并返回环境快照的工具。`perception_radius`（0-32，默认取配置 `block_perception_radius`）裁剪返回的周边方块/实体载荷——默认半径 32 的结果超过 1MB，迭代循环请传小半径 |

## 快捷栏、状态与给予物品

- `get_hotbar` —— 9 个快捷栏槽位（0-8）加 `held_item_slot`；空槽渲染为
  `null`。这是 `set_hotbar_item` / `equip_tool` / `drop_item` 所依赖的
  槽位布局的唯一显式视图。
- `get_bot_status` —— 长耗时操作（`fly_to`、挖掘、`collect_items`）的
  轻量轮询端点：`connected`、`bot_busy`、方块级 + 精确位置、`yaw`、
  生命值与快照时间戳。默认读缓存快照，离线时返回 `connected:false` 而非报错。
- `give_item` —— 冒烟测试的命令兜底模板封装为工具：执行
  `/give <bot> <item> <count>`；`target=hotbar` 时再执行
  `/item replace entity <bot> hotbar.<slot> with <item> <count>`（服务器拒绝
  `/item replace` 时回退到 swap-click 的 `set_hotbar_item` 路径）。
  需要服务器命令权限（OP）。

## 设置与生命周期工具

这四个工具在机器人**离线**时也可用——客户端必须在首次连接之前就能修改 Minecraft 服务器地址：

| 工具 | 说明 |
|------|------|
| `get_settings` | 返回完整配置（MCP 令牌始终脱敏为 `"***"`）以及运行时状态：`online`、`connecting`、`mcp_server_status`。 |
| `update_settings` | 部分更新——只有传入的字段会改变。先校验，再应用到运行中的进程（**没有配置文件**；如需持久化请通过 `MINECRAFT_MCP_*` 环境变量重启）。修改 `mc_address`/`mc_port`/`ai_username` 会在已连接/连接中时触发重连；`mcp_transport`/`mcp_address`/`mcp_port` 在进程重启后生效。 |
| `connect_bot` | 启动到配置服务器的机器人连接。已连接或连接中时为空操作。 |
| `disconnect_bot` | 请求断开连接；重连循环停止。 |

## 错误契约

每个 MCP 错误都携带 JSON-RPC code 以及结构化 `data`（含机器可读的 `reason`、`retryable` 布尔值和各变体专属字段）：

| 错误码 | 变体 | `reason` | retryable |
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

## 说明

- **诚实的错误报告** —— `BotError::InvalidParams` 映射到 MCP
  `INVALID_PARAMS`；不可破坏的方块返回 `MiningInterrupted` 而非 panic；
  `set_game_mode` 会提示需要 OP 权限。
- **命令反馈校验** —— `execute_command` 在发送后会读取服务器的聊天回复：被拒绝
  的命令返回 `CommandRejected` 错误（-32009）并附上服务器的原始反馈，成功命令会
  附带服务器回复（如 "Teleported X to ..."）。`drop_item` 会在点击后校验背包槽位
  是否真的变化，未变化时报 `success: false`。
- **按需获取最新状态** —— `get_self_info` / `get_inventory` 接受 `force=true`
  （默认），读取前会立即触发快照重建。`get_server_info` 通过 `/seed` 实时探测
  `commands_enabled`（缓存至 `refresh=true`）并报告 `bot_busy`。
- **`set_hotbar_item`** —— 通过容器 swap-click 将背包中已有的物品移入快捷栏槽位
  （0-8），是 `/item replace`（语法因服务器而异）的可靠替代方案。物品必须已在
  背包中；不能凭空生成物品。
- **AI 视觉** —— `get_world_view` 渲染附近方块的俯视 PNG 图
  （`mcp/render.rs`）并以 base64 返回，供多模态模型使用。
- **复合操作** —— 在 `compound_ops.rs` 中基于基本命令构建更高层的状态机
  （例如挖取并收集）。
