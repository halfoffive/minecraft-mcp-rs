import type { DefaultTheme, LocaleSpecificConfig } from 'vitepress'

export const en: LocaleSpecificConfig<DefaultTheme.Config> = {
  lang: 'en',
  title: 'Minecraft-MCP-RS',
  description: 'A Minecraft bot controlled via the Model Context Protocol (MCP).',
  themeConfig: {
    nav: [
      { text: 'Home', link: '/', activeMatch: '^/$' },
      { text: 'Guide', link: '/guide/getting-started', activeMatch: '/guide/' },
      { text: 'Tools', link: '/tools', activeMatch: '/tools' },
      { text: 'Config', link: '/config', activeMatch: '/config' },
      { text: 'Development', link: '/dev/building', activeMatch: '/dev/' },
      { text: 'GitHub', link: 'https://github.com/halfoffive/minecraft-mcp-rs' }
    ],

    sidebar: {
      '/guide/': [
        {
          text: 'Guide',
          items: [
            { text: 'Getting Started', link: '/guide/getting-started' },
            { text: 'Architecture', link: '/guide/architecture' }
          ]
        }
      ],
      '/tools/': [
        {
          text: 'Tools',
          items: [{ text: 'Tools', link: '/tools' }]
        }
      ],
      '/config/': [
        {
          text: 'Config',
          items: [{ text: 'Configuration', link: '/config' }]
        }
      ],
      '/npm/': [
        {
          text: 'npm',
          items: [{ text: 'Install via npm', link: '/npm' }]
        }
      ],
      '/dev/': [
        {
          text: 'Development',
          items: [
            { text: 'Building from Source', link: '/dev/building' },
            { text: 'Architecture', link: '/guide/architecture' }
          ]
        }
      ]
    },

    editLink: {
      pattern: 'https://github.com/halfoffive/minecraft-mcp-rs/edit/develop/docs/:path',
      text: 'Edit this page on GitHub'
    },

    footer: {
      message:
        'Released under the MIT License. Not an official Minecraft product, not approved by or associated with Mojang or Microsoft.',
      copyright: 'Copyright © 2024-present Minecraft-MCP-RS contributors'
    },

    docFooter: {
      prev: 'Previous',
      next: 'Next'
    },

    outline: {
      label: 'On this page'
    },

    lastUpdated: {
      text: 'Last updated'
    },

    notFound: {
      title: 'Page Not Found',
      quote:
        "But if you don't change direction, and you keep looking, you might end up where you're heading.",
      linkLabel: 'Go home',
      linkText: 'Take me home'
    },

    search: {
      provider: 'local'
    }
  }
}
