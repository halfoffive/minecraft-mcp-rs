---
name: minecraft-mcp-smoke-test
description: minecraft-mcp-rs 全链路冒烟回归脚本——连接→查询→移动→挖/放/用→战斗→容器→命令→聊天，每大类结束发一条 [SMOKE] 聊天汇总，最后用 get_chat_history 断言。当用户要求对 minecraft-mcp-rs 跑冒烟测试、回归验证 bot 工具链、或复测某次修复时使用。
---

# minecraft-mcp-rs 冒烟测试

一条命令式回归链路，验证 MCP 工具集与真实 Minecraft 1.21.11 服务器的全链路可用性。所有结果通过聊天汇报（send_chat）+ get_chat_history 断言，无需外部测试框架。

## 前置条件

- Minecraft Java **1.21.11** 服务器（azalea 0.15.1 仅支持该版本）
- bot 账号具备 **OP 权限**（execute_command / give_item 需要）且为 **Creative** 模式（fly_to / teleport 需要）
- MCP 客户端已连接（is_connected 返回 connected:true）

## 聊天汇报协议（测试日志）

每个大类完成后立即发一条汇总（send_chat）：

```
[SMOKE] <大类>: <结论>
```

例：`[SMOKE] 查询: OK get_server_info/get_self_info/get_inventory/get_hotbar`

全程不删消息；最后一环用 get_chat_history 读取并逐条核对——每个大类都有 OK 记录即证明全链路无丢失。任何一步失败：先发 `[SMOKE] <大类>: FAIL <原因>` 再排查，不要中断链路（后面的环节可能不依赖失败项）。

## 执行链路

### 0. 连接与查询
1. `is_connected` → connected:true
2. `get_server_info` → 记录 commands_enabled（必须 true 才能走命令环节）与 bot_busy
3. `get_self_info` → 记录 username / position / gamemode / held_item_slot
4. `get_inventory` → 36 槽（含 hotbar 0-8）
5. `get_hotbar` → 9 槽（空槽为 null）+ held_item_slot —— 槽位真相的唯一显式入口
6. `send_chat("[SMOKE] 查询: OK ...")`

### 1. 移动
1. `move_to` 到出生点附近（x±5, y, z±5）
2. `smart_move` 回原点。**首次误报 obstacle 会自动重试一次**（结果含 retried:true）；仍失败才判定真障碍——用 get_nearby_blocks 确认
3. Creative 下 `fly_to` 垂直目标（x, y+5, z）→ reached:true（水平寻路 + 垂直直改坐标）
4. 长耗时移动期间轮询 `get_bot_status`；bot_busy=true 时不要并发发命令
5. `send_chat("[SMOKE] 移动: OK ...")`

### 2. 挖掘 / 放置 / 使用
1. `get_nearby_blocks(filter_type=stone)` 选目标
2. `equip_tool(tool_type=pickaxe)`（主背包工具会自动移入快捷栏）→ `break_block` 或 `act(Mine)`
3. `give_item(item_id=oak_planks, target=hotbar, hotbar_slot=1)` → `place_block(item_slot=1)`
4. 倒水：`give_item(item_id=water_bucket, target=hotbar, hotbar_slot=2)` → `use_item_on_block(item_slot=2)` —— **必须传 item_slot**；结果消息会回显实际使用的物品（应为 water_bucket），对不上就是持握槽位错了
5. `send_chat("[SMOKE] 方块: OK ...")`

### 3. 战斗
1. `give_item(item_id=iron_sword, target=hotbar, hotbar_slot=0)` → `switch_hotbar_slot(0)`
2. `get_nearby_entities(radius=16)` 取实体 id
3. `attack_entity(entity_id)` —— 超出 6 格会自动走近并**重新定位**后再攻击；实体持续移动仍 TooFar 时，按「get_nearby_entities → move_to → attack」循环重试
4. `send_chat("[SMOKE] 战斗: OK ...")`

### 4. 容器
1. 放一个箱子（place_block）→ `open_container` → `put_into_container` → `take_from_container` → `close_container`
2. 容器打开时 `get_inventory` 仍应返回 36 槽快照（历史 bug 已修复，本环节即回归）
3. `send_chat("[SMOKE] 容器: OK ...")`

### 5. 命令
1. `give_item(item_id=diamond_pickaxe, target=inventory)`（/give 兜底模板的标准封装）
2. 备用原始模板：`execute_command("/give <user> minecraft:diamond_pickaxe 1")` + `execute_command("/item replace entity <user> hotbar.0 with minecraft:diamond_pickaxe 1")`
3. `set_hotbar_item(hotbar_slot=0, item_id=diamond_pickaxe)`（swap-click，不依赖 /item replace，任何服务器可用）
4. `execute_command("/time set day")` 验证 OP 命令反馈检测（拒绝时会返回 command_rejected 而不是假成功）
5. `send_chat("[SMOKE] 命令: OK ...")`

### 6. 聊天与断言
1. `send_chat("[SMOKE] 结束: 全链路完成")`
2. `get_chat_history` → 断言 6 条 [SMOKE] 均含 OK，且无消息丢失
3. 向用户汇总结论

## 载荷与效率守则

- `act` 一律带 `perception_radius: 2`（或 0）——默认 32 半径的结果超过 1MB，浪费多模态上下文
- 长耗时操作（fly_to / mine / collect_items）用 `get_bot_status` 轮询（默认读缓存快照，开销极小；position_precise 提供亚方块精度）
- `drop_item` 自带验证（verified:true/false）
- 倒水 / 点火一律传 item_slot，不依赖「当前持握」

## 已知边界

- smart_move：瞬态障碍自动重试一次；retried:true 且仍 obstacle 才是真障碍
- fly_to / teleport：仅 Creative；飞行 = 水平寻路 + 垂直直改坐标（Creative 无摔落伤害）
- attack：6 格外自动靠近；实体持续移动则 TooFar，需重新 get_nearby_entities
- equip_tool：主背包工具自动移入快捷栏（hotbar 满时换入 slot 0，不丢物品）
- 服务器必须 1.21.11
