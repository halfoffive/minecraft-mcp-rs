# Install via npm

The npm distribution lets you run the MCP server with **zero Rust
toolchain** — prebuilt binaries ship in platform packages selected
automatically at install time.

## Quick run (recommended)

Run on demand — `npx` downloads the matching platform binary the first time
and caches it for later runs:

```bash
npx -y minecraft-mcp-rs@1.3.2 --headless --stdio
```

Using [Bun](https://bun.sh)? `bunx` does the same without the `-y` prompt:

```bash
bunx minecraft-mcp-rs@1.3.2 --headless --stdio
```

`--headless` runs without the desktop window and exits the process when the
MCP transport closes; `--stdio` forces the stdio transport.

## Claude Desktop / Cursor config

Add this to your MCP client config:

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

Bun users: swap `"command"` to `bunx` and drop `-y`:

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "bunx",
      "args": ["minecraft-mcp-rs@1.3.2", "--headless", "--stdio"]
    }
  }
}
```

## Global install

If you prefer a permanent install:

```bash
npm install -g minecraft-mcp-rs
minecraft-mcp-rs --headless --stdio
```

## Configuration

There is **no config file** — settings come exclusively from
`MINECRAFT_MCP_*` environment variables (12-factor style). The ones you are
most likely to need:

| Variable | Purpose | Default |
|----------|---------|---------|
| `MINECRAFT_MCP_MC_ADDRESS` | Minecraft server address | `127.0.0.1` |
| `MINECRAFT_MCP_MC_PORT` | Minecraft server port | `25565` |
| `MINECRAFT_MCP_AI_USERNAME` | Bot in-game username | `AI_Bot` |
| `MINECRAFT_MCP_TOKEN` | Pin the HTTP bearer token | random UUID per start |
| `MINECRAFT_MCP_AUTH_ENABLED` | Require bearer auth over HTTP | `false` |

The full table lives on the [Configuration](./config) page. Runtime
changes (`update_settings`, the UI panel) apply to the running process
only; restart with the environment variables to persist them.

## Version compatibility

minecraft-mcp-rs supports **only Minecraft Java Edition 1.21.11**. Other
Minecraft versions are **NOT** supported — pick the minecraft-mcp-rs version
that matches your server:

| Minecraft server version | minecraft-mcp-rs version |
|--------------------------|--------------------------|
| 1.21.11                  | 1.3.2                    |

The `@1.3.2` pins in the snippets above are intentional: a different tool
version may target a different Minecraft version. Always install the exact
release matching your server, and re-check this table before every upgrade.

## Platform packages

The main `minecraft-mcp-rs` package is a thin launcher. The native binary
ships in one of five platform packages listed in `optionalDependencies`:

| Package | Platform |
|---------|----------|
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-x64` | Windows x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-windows-arm64` | Windows arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-darwin-arm64` | macOS arm64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-x64` | Linux x64 |
| `@minecraft-mcp-rs/minecraft-mcp-rs-linux-arm64` | Linux arm64 |

## Troubleshooting

- **"platform package is missing"** — you installed with `--omit=optional`,
  which skips the platform packages. Reinstall with
  `npm install --force`, or install the matching platform package explicitly.
- **Unsupported platform** — the launcher lists the supported
  platform/arch combinations and exits 1.
- **Offline installs** — platform packages are regular npm tarballs; after
  the first successful run they can be reused from the npm cache without a
  network connection. On POSIX systems the launcher self-heals a lost
  executable bit (`EACCES`) by re-applying it once before failing.
