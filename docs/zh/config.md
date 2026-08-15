# 配置

所有设置都有合理的默认值。配置**完全从环境变量读取**（12-factor 风格，同
cargo）——**不再有配置文件**（旧的 `config.json` 与 `--config <path>` 机制
已移除）。

每项设置先取默认值，再被对应的 `MINECRAFT_MCP_*` 环境变量覆盖。非法值只会
记录警告并保留默认值——不会因环境变量拼写错误而启动失败。

| 环境变量 | 字段 | 默认值 | 说明 |
|----------|------|--------|------|
| `MINECRAFT_MCP_MC_ADDRESS` | `mc_address` | `127.0.0.1` | Minecraft 服务器地址 |
| `MINECRAFT_MCP_MC_PORT` | `mc_port` | `25565` | Minecraft 服务器端口 |
| `MINECRAFT_MCP_AI_USERNAME` | `ai_username` | `AI_Bot` | 机器人在游戏中的用户名 |
| `MINECRAFT_MCP_MCP_ADDRESS` | `mcp_address` | `127.0.0.1` | MCP HTTP 绑定地址 |
| `MINECRAFT_MCP_MCP_PORT` | `mcp_port` | `3000` | MCP HTTP 端口 |
| `MINECRAFT_MCP_TASK_NAME` | `task_name` | `mining` | UI 中显示的任务名 |
| `MINECRAFT_MCP_CHUNK_SCAN_RADIUS` | `chunk_scan_radius` | `8` | 扫描区块数（1–16） |
| `MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS` | `block_perception_radius` | `32` | 方块感知范围（8–64） |
| `MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS` | `snapshot_interval_ms` | `500` | 世界快照间隔（毫秒）；空闲时自动放宽到至少 5000 毫秒 |
| `MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS` | `reconnect_initial_delay_ms` | `5000` | 初始重连退避（毫秒） |
| `MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS` | `reconnect_max_delay_ms` | `60000` | 最大重连退避（毫秒） |
| `MINECRAFT_MCP_COMMAND_TIMEOUT_SECS` | `command_timeout_secs` | `30` | 机器人命令超时（秒） |
| `MINECRAFT_MCP_FLY_TIMEOUT_SECS` | `fly_timeout_secs` | `60` | `fly_to` 长距离飞行超时（秒） |
| `MINECRAFT_MCP_TOKEN` | `mcp_token` | 随机 UUID v4 | HTTP 传输的 Bearer Token；固定它的唯一途径 |
| `MINECRAFT_MCP_AUTH_ENABLED` | `mcp_auth_enabled` | `false` | 是否要求 HTTP Bearer Token（默认关闭） |
| `MINECRAFT_MCP_TRANSPORT` | `mcp_transport` | `http` | MCP 传输方式：`stdio` 或 `http` |
| `MINECRAFT_MCP_LANGUAGE` | `language` | 系统语言 | UI 语言：`en` 或 `zh_cn` |

示例：

```bash
export MINECRAFT_MCP_MC_ADDRESS=play.example.com
export MINECRAFT_MCP_MC_PORT=25565
export MINECRAFT_MCP_TRANSPORT=stdio
minecraft-mcp-rs --headless
```

> Windows PowerShell：`$env:MINECRAFT_MCP_MC_ADDRESS="play.example.com"`

## 运行时修改

egui 设置面板与 `update_settings` MCP 工具只修改当前进程——不写任何磁盘
文件。如需持久化，请在重启时通过环境变量配置。已连接时修改
`mc_address`/`mc_port`/`ai_username` 会自动触发重连；
`mcp_transport`/`mcp_address`/`mcp_port` 的变更在下次进程重启时生效。

## 无头模式与命令行参数

二进制接受少量命令行参数（由 clap 解析，帮助文本打印到 stderr）：

| 参数 | 说明 |
|------|------|
| `--gui` | 打开桌面 UI |
| `--headless` | 无桌面窗口；自动连接机器人，MCP 传输关闭时退出进程 |
| `--stdio` | 强制使用 MCP stdio 传输（覆盖环境变量配置）；单独使用时隐含无头模式 |
| `-h`, `--help` | 打印用法到 stderr 并退出 0 |
| `-V`, `--version` | 打印版本到 stderr 并退出 0 |

优先级：`--headless` 优先于 `--gui`；`--stdio` 单独使用隐含无头模式；其余
情况运行 GUI。不带任何参数时打印帮助并退出 0（该模式下**不会**启动 MCP
服务器）；用法错误打印到 stderr 并退出 2。

无头模式下机器人启动时自动连接；通过 `update_settings` MCP 工具修改连接
相关字段会自动重连。`mcp_transport`/`mcp_address`/`mcp_port` 的变更在下次
进程重启时生效。
