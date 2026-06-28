import { defineConfig } from 'vitepress'
import { en } from './en'
import { zh } from './zh'

// Base path for GitHub Pages project site (https://<user>.github.io/<repo>/).
// Override via BASE_PATH env var for forks / custom deployment.
const base = process.env.BASE_PATH || '/minecraft-mcp-rs/'

export default defineConfig({
  base,
  locales: {
    root: { label: 'English', ...en },
    zh: { label: '简体中文', ...zh }
  }
})
