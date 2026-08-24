# 通过 npm 安装

npm 发行版让你**无需 Rust 工具链**即可运行 MCP 服务器——预编译二进制随平台包发布，安装时自动选择。

## 快速运行（推荐）

按需运行——`npx` 首次运行时自动下载匹配平台的二进制并缓存，供后续运行复用：

```bash
npx -y minecraft-mcp-rs@1.4.1 --headless --stdio
```

使用 [Bun](https://bun.sh)？`bunx` 无需 `-y` 参数即可完成同样的事：

```bash
bunx minecraft-mcp-rs@1.4.1 --headless --stdio
```

`--headless` 表示无桌面窗口运行，并在 MCP 传输关闭时退出进程；`--stdio` 强制使用 stdio 传输。

## Claude Desktop / Cursor 配置

在 MCP 客户端配置中加入：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.4.1", "--headless", "--stdio"]
    }
  }
}
```

Bun 用户：把 `"command"` 换成 `bunx`，去掉 `-y`：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "bunx",
      "args": ["minecraft-mcp-rs@1.4.1", "--headless", "--stdio"]
    }
  }
}
```

## 全局安装

如果偏好常驻安装：

```bash
npm install -g minecraft-mcp-rs
minecraft-mcp-rs --headless --stdio
```

## 配置

**没有配置文件** —— 设置只来自 `MINECRAFT_MCP_*` 环境变量（12-factor
风格）。最常用的几个：

| 环境变量 | 用途 | 默认值 |
|----------|------|--------|
| `MINECRAFT_MCP_MC_ADDRESS` | Minecraft 服务器地址 | `127.0.0.1` |
| `MINECRAFT_MCP_MC_PORT` | Minecraft 服务器端口 | `25565` |
| `MINECRAFT_MCP_AI_USERNAME` | Bot 游戏内用户名 | `AI_Bot` |
| `MINECRAFT_MCP_TOKEN` | 固定 HTTP Bearer 令牌 | 每次启动随机 UUID |
| `MINECRAFT_MCP_AUTH_ENABLED` | HTTP 强制 Bearer 鉴权 | `false` |

完整表格见[配置](./config)页。运行期修改（`update_settings` 工具、UI
面板）只对当前进程生效；重启后需用环境变量重新指定。

## 版本兼容性

minecraft-mcp-rs **仅支持 Minecraft Java Edition 1.21.11**，其他 Minecraft 版本**均不支持**——请选择与你服务器版本匹配的 minecraft-mcp-rs 版本：

| Minecraft 服务器版本 | minecraft-mcp-rs 版本 |
|----------------------|-----------------------|
| 1.21.11              | 1.4.0                 |

上文代码片段中的 `@1.4.1` 固定是刻意的：不同工具版本可能面向不同 Minecraft 版本。请始终安装与服务器完全匹配的发行版，并在每次升级前核对本表。

## 平台包

主包 `minecraft-mcp-rs` 是一个薄启动器。原生二进制随 `optionalDependencies`
中列出的五个平台包之一发布：

| 包名 | 平台 |
|------|------|
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-x64` | Windows x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-arm64` | Windows arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-darwin-arm64` | macOS arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-x64` | Linux x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-arm64` | Linux arm64 |

## 故障排查

- **"platform package is missing"** —— 你使用了 `--omit=optional` 安装，跳过了平台包。请用 `npm install --force` 重新安装，或显式安装对应的平台包。
- **不支持的平台** —— 启动器会列出支持的平台/架构组合并退出 1。
- **离线安装** —— 平台包就是普通的 npm tarball；首次成功运行后，无需网络即可从 npm 缓存复用。POSIX 系统上，若可执行位丢失（`EACCES`），启动器会自动补一次权限再重试。
