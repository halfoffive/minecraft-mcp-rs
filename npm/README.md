# minecraft-mcp-rs

MCP server that controls a Minecraft bot (azalea) — prebuilt native binaries.

## Usage

```bash
npx -y minecraft-mcp-rs@1.1.2 --headless --stdio
bunx minecraft-mcp-rs@1.1.2 --headless --stdio
```

> This package supports **Minecraft Java Edition 1.21.11 only**. Other Minecraft versions are not supported — see the [root README](https://github.com/halfoffive/minecraft-mcp-rs#readme) for the version compatibility table.

or install globally:

```bash
npm install -g minecraft-mcp-rs
minecraft-mcp-rs --headless --stdio
```

For Claude Desktop / Cursor, add to your MCP client config:

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.1.2", "--headless", "--stdio"]
    }
  }
}
```

> Using bun? Replace `"command": "npx"` with `"command": "bunx"` and drop the
> `-y` flag (bunx installs on demand without prompting).

See the [root README](https://github.com/halfoffive/minecraft-mcp-rs#readme) for
the full documentation (config file, MCP tools, CLI flags).

## Platform packages

The main package is a thin launcher; the native binary ships in one of five
platform packages selected automatically at install time:

| Package | Platform |
| --- | --- |
| `minecraft-mcp-rs-windows-x64` | Windows x64 |
| `minecraft-mcp-rs-windows-arm64` | Windows arm64 |
| `minecraft-mcp-rs-darwin-arm64` | macOS arm64 |
| `minecraft-mcp-rs-linux-x64` | Linux x64 |
| `minecraft-mcp-rs-linux-arm64` | Linux arm64 |

Installing with `--omit=optional` breaks the launcher; reinstall the matching
platform package explicitly to fix it.
