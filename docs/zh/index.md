---
layout: home

hero:
  name: minecraft-mcp-rs
  text: "由 LLM 驱动的 Minecraft 机器人。"
  tagline: "41 个 MCP 工具，把 Claude Desktop、Cursor 或任何 MCP 宿主接入实时的 Minecraft 1.21.11 世界。零 Rust 工具链 —— 一条 npx 命令即可运行。"
  actions:
    - theme: brand
      text: npx 快速运行
      link: /zh/guide/getting-started
    - theme: alt
      text: npm 安装
      link: /zh/npm
    - theme: alt
      text: GitHub
      link: https://github.com/halfoffive/minecraft-mcp-rs

features:
  - title: "41 个 MCP 工具"
    details: "机器人能力被组织为 8 个领域——查询、移动、方块、物品、容器、战斗、聊天与设置——外加统一的 `act` 工具。stdio 或本地回环 HTTP 皆可。"
  - title: "实时世界感知"
    details: "脏区域快照让线程安全的世界视图保持新鲜；两次轮询之间只重读发生变化的方块与区块。"
  - title: "多模态视觉"
    details: "`get_world_view` 渲染带朝向箭头的俯视 PNG，并附带 JSON 标注一起返回 —— 模型看到的就是 bot 看到的。"
  - title: "诚实的错误"
    details: "每个失败都携带机器可读的 reason 与 retryable 提示；放置操作会对照世界验证 —— 绝不假报成功。"
---
