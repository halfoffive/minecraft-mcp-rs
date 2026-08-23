<!-- One-click MCP host setup for the home page.
     Renders install actions for Cursor (deeplink), Claude Code and VS Code
     (copyable CLI commands) plus the raw JSON config for every other MCP host.
     Styling references VitePress default CSS variables ONLY — no new colors. -->
<script setup>
import { computed, ref } from 'vue'
import { useData } from 'vitepress'

const SERVER_NAME = 'minecraft-mcp-rs'
const SERVER_ARGS = ['-y', 'minecraft-mcp-rs@latest', '--headless', '--stdio']

// Config shape shared by every host (see docs/npm.md).
const serverConfig = { name: SERVER_NAME, command: 'npx', args: SERVER_ARGS }
const jsonConfig = JSON.stringify(
  { mcpServers: { [SERVER_NAME]: { command: 'npx', args: SERVER_ARGS } } },
  null,
  2
)

// Official documented forms:
// - Claude Code: `claude mcp add <name> -- <command> [args…]` (`--` guards the -y flag)
// - VS Code: `code --add-mcp '<json>'` with escaped quotes, as shown in the VS Code docs
const claudeCodeCmd = `claude mcp add ${SERVER_NAME} -- npx ${SERVER_ARGS.join(' ')}`
const vscodeCmd = `code --add-mcp "${JSON.stringify(serverConfig).replace(/"/g, '\\"')}"`

// Cursor deeplink: base64url(no padding) of the server config JSON.
function toBase64Url(value) {
  const bytes = new TextEncoder().encode(JSON.stringify(value))
  let binary = ''
  bytes.forEach((b) => (binary += String.fromCharCode(b)))
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}
const cursorLink = `cursor://anysphere.cursor-deeplink/mcp/install?name=${SERVER_NAME}&config=${toBase64Url(serverConfig)}`

const { lang } = useData()
const t = computed(() =>
  lang.value.startsWith('zh')
    ? {
        title: '一键接入你的 MCP 宿主',
        hint: '无需 Rust 工具链 —— 选择宿主即可安装，或复制 JSON 手动配置。',
        cursor: '添加到 Cursor',
        claudeCode: '复制 Claude Code 命令',
        vscode: '复制 VS Code 命令',
        json: '复制 MCP JSON',
        copied: '已复制 ✓',
        manualTitle: '手动配置（Claude Desktop / 其他 MCP 宿主）'
      }
    : {
        title: 'One-click setup for your MCP host',
        hint: 'No Rust toolchain required — pick your host, or copy the JSON manually.',
        cursor: 'Add to Cursor',
        claudeCode: 'Copy Claude Code command',
        vscode: 'Copy VS Code command',
        json: 'Copy MCP JSON',
        copied: 'Copied ✓',
        manualTitle: 'Manual config (Claude Desktop / any other MCP host)'
      }
)

const copied = ref('')
let resetTimer = null
async function copy(text, key) {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    // Fallback for non-secure contexts (e.g. LAN preview).
    const textarea = document.createElement('textarea')
    textarea.value = text
    document.body.appendChild(textarea)
    textarea.select()
    document.execCommand('copy')
    textarea.remove()
  }
  copied.value = key
  clearTimeout(resetTimer)
  resetTimer = setTimeout(() => (copied.value = ''), 2000)
}
</script>

<template>
  <div class="mcp-quick-setup">
    <h2 class="setup-title">{{ t.title }}</h2>
    <p class="setup-hint">{{ t.hint }}</p>
    <div class="setup-actions">
      <a class="setup-btn primary" :href="cursorLink">
        <span>{{ t.cursor }}</span>
      </a>
      <button class="setup-btn" type="button" @click="copy(claudeCodeCmd, 'claude')">
        {{ copied === 'claude' ? t.copied : t.claudeCode }}
      </button>
      <button class="setup-btn" type="button" @click="copy(vscodeCmd, 'vscode')">
        {{ copied === 'vscode' ? t.copied : t.vscode }}
      </button>
      <button class="setup-btn" type="button" @click="copy(jsonConfig, 'json')">
        {{ copied === 'json' ? t.copied : t.json }}
      </button>
    </div>
    <details class="setup-manual">
      <summary>{{ t.manualTitle }}</summary>
      <div class="vp-code-block">
        <pre><code>{{ jsonConfig }}</code></pre>
      </div>
    </details>
  </div>
</template>

<style scoped>
.mcp-quick-setup {
  margin-top: 2rem;
  padding: 1.25rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  background: var(--vp-c-bg-soft);
}
.setup-title {
  margin: 0 0 0.5rem;
  border-top: none;
  font-size: 1.15rem;
}
.setup-hint {
  margin: 0 0 1rem;
  color: var(--vp-c-text-2);
  font-size: 0.9rem;
}
.setup-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.6rem;
}
.setup-btn {
  display: inline-flex;
  align-items: center;
  padding: 0.4rem 1rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  font-size: 0.9rem;
  cursor: pointer;
  transition: border-color 0.25s, color 0.25s;
}
.setup-btn:hover {
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
}
.setup-btn.primary {
  text-decoration: none;
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
  font-weight: 600;
}
.setup-manual {
  margin-top: 1rem;
  font-size: 0.9rem;
}
.setup-manual summary {
  cursor: pointer;
  color: var(--vp-c-text-2);
}
.vp-code-block {
  margin-top: 0.5rem;
  padding: 0.75rem 1rem;
  border-radius: 8px;
  background: var(--vp-c-bg-elv);
  overflow-x: auto;
}
.vp-code-block pre {
  margin: 0;
}
</style>
