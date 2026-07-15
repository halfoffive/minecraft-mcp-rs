# 配置

所有设置都有合理的默认值，并可在运行时通过 egui 设置面板修改 —— 完全可编辑，
字符串使用文本输入框，数值字段使用 `DragValue` 滑块。编辑完成后，点击
**Connect** 即可应用设置并在专用的后台线程上启动机器人连接。

| 字段 | 默认值 | 说明 |
|-------|---------|-------------|
| `mc_address` | `127.0.0.1` | Minecraft 服务器地址 |
| `mc_port` | `25565` | Minecraft 服务器端口 |
| `ai_username` | `AI_Bot` | 机器人在游戏中的用户名 |
| `mcp_transport` | `Http` | MCP 传输方式：`Stdio` 或 `Http` |
| `mcp_address` | `127.0.0.1` | MCP HTTP 绑定地址（仅限回环） |
| `mcp_port` | `3000` | MCP HTTP 端口 |
| `mcp_token` | `minecraft-mcp-rs` | HTTP 传输的 Bearer 令牌 |
| `chunk_scan_radius` | `8` | 扫描的区块数（1–16） |
| `block_perception_radius` | `32` | 方块感知范围（8–64） |
| `snapshot_interval_ms` | `500` | 世界快照间隔 |
| `reconnect_initial_delay_ms` | `5000` | 初始重连退避时间 |
| `reconnect_max_delay_ms` | `60000` | 最大重连退避时间 |
| `command_timeout_secs` | `30` | 机器人命令超时时间 |

## 日志

所有日志输出仅写入 **stderr** —— stdout 保留给 MCP JSON-RPC 传输使用。
默认过滤级别为：`minecraft_mcp_rs=debug, azalea=warn`。
