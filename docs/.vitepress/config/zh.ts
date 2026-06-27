import type { DefaultTheme, LocaleSpecificConfig } from 'vitepress'

// NOTE: The GitHub owner/org below is a placeholder (`your-org`). Update it to
// the real repository owner before publishing docs.
export const zh: LocaleSpecificConfig<DefaultTheme.Config> = {
  lang: 'zh-CN',
  title: 'minecraft-mcp-rs',
  description: '通过模型上下文协议（MCP）控制的 Minecraft 机器人。',
  themeConfig: {
    nav: [
      { text: '指南', link: '/zh/guide/getting-started', activeMatch: '/zh/guide/' },
      { text: '工具', link: '/zh/guide/tools', activeMatch: '/zh/guide/' },
      { text: '配置', link: '/zh/guide/configuration' },
      { text: 'GitHub', link: 'https://github.com/your-org/minecraft-mcp-rs' }
    ],

    sidebar: {
      '/zh/guide/': [
        {
          text: '指南',
          items: [
            { text: '入门指南', link: '/zh/guide/getting-started' },
            { text: '配置', link: '/zh/guide/configuration' },
            { text: '工具', link: '/zh/guide/tools' },
            { text: '架构', link: '/zh/guide/architecture' }
          ]
        }
      ]
    },

    editLink: {
      pattern: 'https://github.com/your-org/minecraft-mcp-rs/edit/main/docs/:path',
      text: '在 GitHub 上编辑此页'
    },

    footer: {
      message: '基于 MIT 许可发布。',
      copyright: '版权所有 © 2024-至今 minecraft-mcp-rs 贡献者'
    },

    docFooter: {
      prev: '上一页',
      next: '下一页'
    },

    outline: {
      label: '本页目录'
    },

    lastUpdated: {
      text: '最后更新于'
    },

    notFound: {
      title: '页面未找到',
      quote:
        '但如果你不改变方向，并且继续寻找，你可能最终会到达你所前往的地方。',
      linkLabel: '前往首页',
      linkText: '带我回首页'
    },

    search: {
      provider: 'local'
    }
  }
}
