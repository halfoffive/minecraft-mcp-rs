---
layout: home

hero:
  name: minecraft-mcp-rs
  text: "A Minecraft bot controlled via the Model Context Protocol (MCP)."
  tagline: "Bridge an LLM client to a live Minecraft world."
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/your-org/minecraft-mcp-rs

features:
  - title: "30+ MCP Tools"
    details: "Bot abilities organized into 7 domains — query, movement, block, item, container, combat, and chat — plus a unified `act` tool. Exposed over stdio or remote HTTP."
  - title: "Live World State"
    details: "The bot periodically snapshots its surroundings into a thread-safe `SharedState`, readable by all tools. Only changed blocks/chunks are recomputed between polling intervals."
  - title: "Remote HTTP Server"
    details: "A loopback-only (`127.0.0.1`) MCP HTTP server, Bearer-token protected. Transport mode is selectable in the UI; a copyable JSON config is generated for Claude Desktop / Cursor."
  - title: "AI Vision"
    details: "The `get_world_view` tool renders a top-down PNG of nearby blocks and returns it as base64, letting multimodal models reason about the world visually."
  - title: "Smart Movement & Flight"
    details: "`smart_move` auto-jumps over 1-block gaps and stops on larger obstacles; `fly_to` flies to a target in creative mode. Compound operations compose primitive commands."
  - title: "Desktop UI"
    details: "An egui/eframe desktop app with a live status panel, fully editable settings, and an MCP Config panel — alongside auto-reconnect with exponential backoff."
---
