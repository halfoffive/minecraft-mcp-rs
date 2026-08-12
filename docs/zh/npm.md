# 通过 npm 安装

npm 发行版让你**无需 Rust 工具链**即可运行 MCP 服务器——预编译二进制随平台包发布，安装时自动选择。

## 安装

全局安装：

```bash
npm install -g minecraft-mcp-rs
```

或按需运行（首次运行时会下载匹配平台的二进制）：

```bash
npx -y minecraft-mcp-rs@1.1.4 --headless --stdio
bunx minecraft-mcp-rs@1.1.4 --headless --stdio
```

## Claude Desktop / Cursor 配置

在 MCP 客户端配置中加入：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.1.4", "--headless", "--stdio"]
    }
  }
}
```

`--headless` 表示无桌面窗口运行，并在 MCP 传输关闭时退出进程；`--stdio` 强制使用 stdio 传输。使用 bun 时，命令是 `bunx`（它会自动安装包，无需 `-y` 参数）：

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "bunx",
      "args": ["minecraft-mcp-rs@1.1.4", "--headless", "--stdio"]
    }
  }
}
```

## 版本兼容性

minecraft-mcp-rs **仅支持 Minecraft Java Edition 1.21.11**，其他 Minecraft 版本**均不支持**——请选择与你服务器版本匹配的 minecraft-mcp-rs 版本：

| Minecraft 服务器版本 | minecraft-mcp-rs 版本 |
|----------------------|-----------------------|
| 1.21.11              | 1.1.4                 |

上面示例中的 `@1.1.4` 是刻意固定的版本号：不同的工具版本可能对应不同的 Minecraft 版本。请始终安装与你服务器版本匹配的发行版，并在每次升级前重新核对此表。

## 平台包

主包 `minecraft-mcp-rs` 只是一个启动器（launcher）。原生二进制位于 `optionalDependencies` 中列出的五个平台包之一：

| 包名 | 平台 |
|---------|----------|
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-x64` | Windows x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-arm64` | Windows arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-darwin-arm64` | macOS arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-x64` | Linux x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-arm64` | Linux arm64 |

## 故障排查

- **"platform package is missing"** —— 你使用了 `--omit=optional` 安装，跳过了平台包。请用 `npm install --force` 重新安装，或显式安装对应的平台包（例如 `npm install minecraft-mcp-rs-linux-x64`）。
- **不支持的平台** —— 启动器会列出支持的平台/架构组合并退出 1。
- **离线安装** —— 平台包就是普通的 npm tarball；首次成功安装后，无需网络即可从 npm 缓存复用。

## 配置文件

二进制会在系统配置目录读写 `config.json`（Windows 为 `%APPDATA%\minecraft-mcp-rs\`，Linux 为 `~/.config/minecraft-mcp-rs/`，macOS 为 `~/Library/Application Support/minecraft-mcp-rs/`）。可用 `--config <path>` 指定其他位置。
