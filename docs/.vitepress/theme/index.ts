// Custom VitePress theme: extends the default theme with the project's
// "industrial block manual" styling (see custom.css). Colors are untouched —
// every rule references VitePress default CSS variables only.
import DefaultTheme from 'vitepress/theme'
import McpQuickSetup from './components/McpQuickSetup.vue'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('McpQuickSetup', McpQuickSetup)
  }
}
