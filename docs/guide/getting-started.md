# Getting Started

`minecraft-mcp-rs` is a Minecraft bot controlled via the Model Context
Protocol (MCP). This page gets you from zero to a connected bot.

## Run without Rust (recommended)

No toolchain needed — `npx` (or `bunx`) downloads the prebuilt binary for
your platform on first use:

```bash
npx -y minecraft-mcp-rs@1.3.2 --headless --stdio
# or
bunx minecraft-mcp-rs@1.3.2 --headless --stdio
```

Point your MCP client at it (Claude Desktop / Cursor):

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

Bun users: swap `"command": "npx"` for `"command": "bunx"` and drop `-y`.
Full details — global install, platform packages, troubleshooting — live on
the [npm install](../npm) page.

> Only **Minecraft Java Edition 1.21.11** is supported — no other server
> version works. Always pin the `minecraft-mcp-rs` release matching your
> server (see [version compatibility](../npm#version-compatibility)).

| Minecraft server version | minecraft-mcp-rs version |
|--------------------------|--------------------------|
| 1.21.11                  | 1.3.2                    |

## Configure

The server reads its settings exclusively from `MINECRAFT_MCP_*`
environment variables — e.g. point the bot at your server before starting:

```bash
MINECRAFT_MCP_MC_ADDRESS=mc.example.com \
MINECRAFT_MCP_AI_USERNAME=AI_Bot \
npx -y minecraft-mcp-rs@1.3.2 --headless --stdio
```

Every variable is listed on the [Configuration](../config) page. Settings
changed at runtime (`update_settings`, the UI panel) apply to the running
process only; restart with the environment variables to persist them.

## Build from source

If you want to hack on the bot itself, head to
[Building from Source](../dev/building) — prerequisites, the pinned-nightly
rationale, the full test/lint gate suite, and common build issues.
