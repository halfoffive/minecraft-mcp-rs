---
layout: home

hero:
  name: minecraft-mcp-rs
  text: "A Minecraft bot, driven by your LLM."
  tagline: "41 MCP tools bridging Claude Desktop, Cursor or any MCP host to a live Minecraft 1.21.11 world. Zero Rust toolchain — one npx command."
  actions:
    - theme: brand
      text: Quick Run with npx
      link: /guide/getting-started
    - theme: alt
      text: Install via npm
      link: /npm
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
