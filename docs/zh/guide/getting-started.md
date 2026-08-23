# 入门指南

`minecraft-mcp-rs` 是一个通过模型上下文协议（MCP）控制的 Minecraft 机器人。
本页带你从零开始连上第一个 bot。

## 免 Rust 直接运行（推荐）

无需任何工具链 —— `npx`（或 `bunx`）首次运行时自动下载对应平台的预编译
二进制：

```bash
npx -y minecraft-mcp-rs@1.3.2 --headless --stdio
# 或者
bunx minecraft-mcp-rs@1.3.2 --headless --stdio
```

在 MCP 客户端中接入（Claude Desktop / Cursor）：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.3.2", "--headless", "--stdio"]
    }
  }
}
```

Bun 用户：把 `"command": "npx"` 换成 `"command": "bunx"`，并去掉 `-y`。
全局安装、平台包、故障排查等完整说明见 [npm 安装](../npm) 页面。

> 本工具**仅支持 Minecraft Java Edition 1.21.11**，其他服务器版本均无法
> 工作。请始终固定与服务器匹配的 `minecraft-mcp-rs` 版本（参见
> [版本兼容性](../npm#版本兼容性)）。

| Minecraft 服务器版本 | minecraft-mcp-rs 版本 |
|----------------------|-----------------------|
| 1.21.11              | 1.3.2                 |

## 配置

服务器的设置**只**来自 `MINECRAFT_MCP_*` 环境变量 —— 例如启动前把 bot
指向你的服务器：

```bash
MINECRAFT_MCP_MC_ADDRESS=mc.example.com \
MINECRAFT_MCP_AI_USERNAME=AI_Bot \
npx -y minecraft-mcp-rs@1.3.2 --headless --stdio
```

全部变量见[配置](../config)页。运行期修改的设置（`update_settings`
工具、UI 面板）只对当前进程生效；重启后需用环境变量重新指定。

## 从源码构建

想参与开发的话，请看[从源码构建](../dev/building)—— 前置条件、固定
nightly 的原因、完整的测试/lint 门禁套件以及常见构建问题。
