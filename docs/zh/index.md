---
layout: home

hero:
  name: minecraft-mcp-rs
  text: "通过模型上下文协议（MCP）控制的 Minecraft 机器人。"
  tagline: "将 LLM 客户端连接到实时的 Minecraft 世界。"
  actions:
    - theme: brand
      text: 快速开始
      link: /zh/guide/getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/your-org/minecraft-mcp-rs

features:
  - title: "30+ MCP 工具"
    details: "机器人能力被组织为 7 个领域——查询、移动、方块、物品、容器、战斗与聊天——外加统一的 `act` 工具。通过 stdio 或远程 HTTP 暴露。"
  - title: "实时世界状态"
    details: "机器人定期将其周围环境快照到线程安全的 `SharedState` 中，所有工具均可读取。两次轮询之间仅重新计算发生变化的方块/区块。"
  - title: "远程 HTTP 服务器"
    details: "仅限回环地址（`127.0.0.1`）的 MCP HTTP 服务器，使用 Bearer 令牌保护。传输模式可在 UI 中切换；并为 Claude Desktop / Cursor 生成可复制的 JSON 配置。"
  - title: "AI 视觉"
    details: "`get_world_view` 工具渲染附近方块的俯视 PNG 图并以 base64 返回，让多模态模型能够从视觉上理解世界。"
  - title: "智能移动与飞行"
    details: "`smart_move` 会自动跳过 1 格间隙并在更大的障碍前停下；`fly_to` 在创造模式下飞向目标。复合操作由基本命令组合而成。"
  - title: "桌面 UI"
    details: "基于 egui/eframe 的桌面应用，包含实时状态面板、可完整编辑的设置以及 MCP 配置面板——并带有指数退避的自动重连。"
---
