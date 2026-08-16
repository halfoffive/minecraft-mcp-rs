# Install via npm

The npm distribution lets you run the MCP server with **zero Rust toolchain** —
prebuilt binaries ship in platform packages selected automatically at install
time.

## Install

Install globally:

```bash
npm install -g minecraft-mcp-rs
```

or run it on demand (downloads the matching platform binary the first time):

```bash
npx -y minecraft-mcp-rs@1.3.1 --headless --stdio
bunx minecraft-mcp-rs@1.3.1 --headless --stdio
```

## Claude Desktop / Cursor config

Add this to your MCP client config:

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@1.3.1", "--headless", "--stdio"]
    }
  }
}
```

Using Bun instead? Swap the command to `bunx` and drop the `-y` flag (bunx
installs and runs packages without prompting):

```json
{
  "mcpServers": {
    "minecraft": {
      "command": "bunx",
      "args": ["minecraft-mcp-rs@1.3.1", "--headless", "--stdio"]
    }
  }
}
```

`--headless` runs without the desktop window and exits the process when the
MCP transport closes; `--stdio` forces the stdio transport.

## Version compatibility

minecraft-mcp-rs supports **only Minecraft Java Edition 1.21.11**. Other
Minecraft versions are **NOT** supported — pick the minecraft-mcp-rs version
that matches your server:

| Minecraft server version | minecraft-mcp-rs version |
|--------------------------|--------------------------|
| 1.21.11                  | 1.3.1                    |

The `@1.3.1` pins in the snippets above are intentional: a different tool
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
  `npm install --force`, or install the matching platform package explicitly
  (e.g. `npm install minecraft-mcp-rs-linux-x64`).
- **Unsupported platform** — the launcher lists the supported
  platform/arch combinations and exits 1.
- **Offline installs** — platform packages are regular npm tarballs; after the
  first successful install they can be reused from the npm cache without a
  network connection.

## Config file

The binary reads/writes `config.json` in the OS config dir
(`%APPDATA%\minecraft-mcp-rs\` on Windows, `~/.config/minecraft-mcp-rs/` on
Linux, `~/Library/Application Support/minecraft-mcp-rs/` on macOS). Point it
elsewhere with `--config <path>`.
