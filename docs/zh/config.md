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
| `mcp_token` | 随机 UUID v4 | HTTP 传输的 Bearer 令牌（可选鉴权，每次 `AppConfig::default()` 时随机生成） |
| `mcp_auth_enabled` | `false` | 需要 Bearer Token 鉴权（默认关闭） |
| `chunk_scan_radius` | `8` | 扫描的区块数（1–16） |
| `block_perception_radius` | `32` | 方块感知范围（8–64） |
| `snapshot_interval_ms` | `500` | 世界快照间隔 |
| `reconnect_initial_delay_ms` | `5000` | 初始重连退避时间 |
| `reconnect_max_delay_ms` | `60000` | 最大重连退避时间 |
| `command_timeout_secs` | `30` | 机器人命令超时时间 |

## 配置文件持久化

设置会持久化到系统配置目录下的 `config.json`，并在每次启动时重新加载——文件中的值会覆盖默认值：

| 系统 | 路径 |
|----|------|
| Windows | `%APPDATA%\minecraft-mcp-rs\config.json` |
| Linux | `~/.config/minecraft-mcp-rs/config.json` |
| macOS | `~/Library/Application Support/minecraft-mcp-rs/config.json` |

写入是原子的（临时文件 + 重命名，Unix 下权限为 `0600`）。可用 `--config <path>` 指定其他文件。

## 无头模式与命令行参数

二进制接受少量命令行参数：

| 参数 | 说明 |
|------|-------------|
| `--headless` | 无桌面窗口；自动连接机器人，MCP 传输关闭时退出进程 |
| `--stdio` | 强制使用 MCP stdio 传输（覆盖配置）；单独使用时隐含无头模式 |
| `--gui` | 打开桌面 UI（显式指定时优先于 `--stdio` 隐含的无头模式） |
| `--config <path>` | 从指定路径加载配置文件 |
| `-h`, `--help` | 打印用法到 stderr 并退出 |

无参数时打印帮助并退出 0；`--gui` 打开桌面 UI。

无头模式下机器人启动时自动连接；通过 `update_settings` MCP 工具修改服务器连接相关字段会自动触发重连。`mcp_transport`/`mcp_address`/`mcp_port` 的变更在下次进程重启时生效。

## 日志

所有日志输出仅写入 **stderr** —— stdout 保留给 MCP JSON-RPC 传输使用。
默认过滤级别为：`minecraft_mcp_rs=debug, azalea=warn`。
