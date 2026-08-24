---
layout: home

hero:
  name: Minecraft-MCP-RS
  text: "A Minecraft bot, driven by your LLM."
  tagline: "41 MCP tools bridging Claude Desktop, Cursor or any MCP host to a live Minecraft 1.21.11 world. Zero Rust toolchain — one npx command."
  image:
    src: /logo.png
    alt: Minecraft-MCP-RS
  actions:
    - theme: brand
      text: Quick Run with npx
      link: /guide/getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/halfoffive/minecraft-mcp-rs

features:
  - title: "41 MCP Tools"
    details: "Bot abilities organized into 8 domains — query, movement, block, item, container, combat, chat, and settings — plus a unified `act` tool. Over stdio or loopback HTTP."
  - title: "Live World Perception"
    details: "Dirty-region snapshots keep a thread-safe view of the world fresh; only changed blocks and chunks are re-read between polls."
  - title: "Vision for Multimodal LLMs"
    details: "`get_world_view` renders a top-down PNG with a yaw heading arrow and returns it beside a JSON annotation — the model sees what the bot sees."
  - title: "Honest Errors"
    details: "Every failure carries a machine-readable reason and retryable hint; placements are verified against the world — never fake successes."
---

<McpQuickSetup />

## Disclaimer

Minecraft-MCP-RS is an **unofficial** third-party tool and is **not affiliated with, endorsed by, or sponsored by Mojang Studios or Microsoft**. "Minecraft" is a trademark of Mojang Synergies AB; this project merely enables programmatic access to a Minecraft server you own and operate. Use of the bot must comply with the rules of any server you connect it to, including anti-cheat and automation policies — unauthorized automation on servers you do not control (e.g. public servers prohibiting bots) may violate those servers' terms and result in sanctions against your account. This software is provided under the MIT License, without warranty of any kind.
