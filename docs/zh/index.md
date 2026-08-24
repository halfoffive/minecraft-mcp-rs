---
layout: home

hero:
  name: Minecraft-MCP-RS
  text: "由 LLM 驱动的 Minecraft 机器人。"
  tagline: "41 个 MCP 工具，把 Claude Desktop、Cursor 或任何 MCP 宿主接入实时的 Minecraft 1.21.11 世界。零 Rust 工具链 —— 一条 npx 命令即可运行。"
  image:
    src: /logo.png
    alt: Minecraft-MCP-RS
  actions:
    - theme: brand
      text: npx 快速运行
      link: /zh/guide/getting-started
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

## 一键接入主流 Agent

无需 Rust 工具链 —— 把下面的配置加入你的 MCP 客户端配置文件(Claude Desktop、Cursor、Cline、Codex 等任何 MCP 宿主)即可:

```json
{
  "mcpServers": {
    "minecraft-mcp-rs": {
      "command": "npx",
      "args": ["-y", "minecraft-mcp-rs@latest", "--headless", "--stdio"]
    }
  }
}
```

## 免责声明

Minecraft-MCP-RS 是**非官方**第三方工具，与 Mojang Studios 或 Microsoft **无任何隶属、认可或赞助关系**。"Minecraft" 是 Mojang Synergies AB 的商标；本项目仅用于对你拥有并运营的 Minecraft 服务器进行程序化访问。使用本机器人须遵守所连接服务器的规则（包括反作弊与自动化策略）——在非你控制的服务器（如禁止机器人的公共服务器）上未经授权使用自动化，可能违反该服务器条款并导致账号受罚。本软件基于 MIT 许可证提供，不附带任何形式的担保。
