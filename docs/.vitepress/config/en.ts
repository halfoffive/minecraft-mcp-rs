import type { DefaultTheme, LocaleSpecificConfig } from 'vitepress'

// NOTE: The GitHub owner/org below is a placeholder (`your-org`). Update it to
// the real repository owner before publishing docs.
export const en: LocaleSpecificConfig<DefaultTheme.Config> = {
  lang: 'en',
  title: 'minecraft-mcp-rs',
  description: 'A Minecraft bot controlled via the Model Context Protocol (MCP).',
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/getting-started', activeMatch: '/guide/' },
      { text: 'Tools', link: '/guide/tools', activeMatch: '/guide/' },
      { text: 'Config', link: '/guide/configuration' },
      { text: 'GitHub', link: 'https://github.com/your-org/minecraft-mcp-rs' }
    ],

    sidebar: {
      '/guide/': [
        {
          text: 'Guide',
          items: [
            { text: 'Getting Started', link: '/guide/getting-started' },
            { text: 'Configuration', link: '/guide/configuration' },
            { text: 'Tools', link: '/guide/tools' },
            { text: 'Architecture', link: '/guide/architecture' }
          ]
        }
      ]
    },

    editLink: {
      pattern: 'https://github.com/your-org/minecraft-mcp-rs/edit/main/docs/:path',
      text: 'Edit this page on GitHub'
    },

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2024-present minecraft-mcp-rs contributors'
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
