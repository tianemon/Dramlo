<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

defineProps<{ intro?: boolean }>()

const mcpCommand = ref('')
const mcpWorkspace = ref('')
const mcpFfmpegBin = ref('')
const launcherToken = ref(false)
const showConfig = ref(false)
const mcpConfig = computed(() => JSON.stringify({
  mcpServers: {
    'dramlo': {
      command: mcpCommand.value,
      args: ['mcp', 'studio-compact', ...(launcherToken.value ? ['--token-from-launcher'] : [])],
      env: {
        DRAMLO_WEB_MCP_URL: location.origin,
        DRAMLO_WEB_WORKSPACE: mcpWorkspace.value,
        ...(mcpFfmpegBin.value ? { DRAMLO_FFMPEG_BIN: mcpFfmpegBin.value } : {})
      }
    }
  }
}, null, 2))
const copyStatus = ref('')
const showSetup = ref(false)
const setupText = computed(() => {
  const studio = [
    '命令：' + (mcpCommand.value || '当前 Dramlo 发行包内 dramlo-server.exe 的绝对路径，请先定位该文件。'),
    '参数：mcp、studio-compact' + (launcherToken.value ? '、--token-from-launcher' : ''),
    'DRAMLO_WEB_MCP_URL=' + location.origin,
    'DRAMLO_WEB_WORKSPACE=' + (mcpWorkspace.value || '所选工作区的绝对路径'),
    launcherToken.value
      ? '桌面启动版由同一 Windows 用户通过 --token-from-launcher 使用本机加密令牌，配置中无需填写令牌。'
      : '手动启动版请通过客户端的环境变量或密钥配置提供与服务相同的 DRAMLO_MCP_BRIDGE_TOKEN。'
  ]
  return [
    '# 接入本机 Dramlo',
    '请按当前 Agent 客户端的方式配置下方唯一的 Dramlo stdio MCP。已有 Dramlo 连接时复用原配置。',
    '## Dramlo 工作台',
    ...studio,
    '调用 studio_web_status 检查连接。离线时用 studio_find 查找 studio_start_web，再用 studio_call 启动工作区。连接后用同样方式按需读取 studio_get_guide 的指南章节。',
    'ComfyUI 发现、管理和导入已包含在这条连接里。接通后默认调用 studio_comfy_discover 做只读发现；若未发现本机安装，继续完成 Dramlo 接入，有需要时再请用户提供目录。',
    '用发现结果的 target 直接查看工作流和槽位。用户选定工作流后，通过同一 target 导入。需要长期管理或托管启停时才保存实例。',
    '影视短剧创作前先用 studio_list_skills 查看 Dramlo 技能库，再用 studio_read_skill 直接读取匹配技能及参考资料，在当前任务中使用。禁止把 Dramlo Skill 安装、复制或注册到外部 Agent 的环境。'
  ].join('\n')
})

const handoff = computed(() => mcpCommand.value && mcpWorkspace.value
  ? setupText.value + '\n\n# MCP 客户端 JSON 配置\n\n' + mcpConfig.value
  : setupText.value)

onMounted(async () => {
  try {
    const response = await fetch('/api/dramlo/session', { headers: { 'X-Dramlo-UI': '1' } })
    if (response.ok) {
      const session = await response.json()
      if (typeof session.mcpCommand === 'string') mcpCommand.value = session.mcpCommand
      if (typeof session.root === 'string') mcpWorkspace.value = session.root
      if (typeof session.mcpFfmpegBin === 'string') mcpFfmpegBin.value = session.mcpFfmpegBin
      launcherToken.value = session.mcpLauncherToken === true
    }
  } catch {
    // The guide can still be used with a manually supplied executable path.
  }
})

async function copy(content: string, label: string) {
  try {
    await navigator.clipboard.writeText(content)
    copyStatus.value = '已复制' + label
  } catch {
    copyStatus.value = '无法访问剪贴板，请选中下方正文手动复制。'
  }
}

function copySetup() {
  showSetup.value = true
  void copy(handoff.value, '连接说明')
}
function copyConfig() {
  showConfig.value = true
  void copy(mcpConfig.value, 'MCP 配置')
}
function download(filename: string, content: string) {
  const url = URL.createObjectURL(new Blob([content], { type: 'text/markdown;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
</script>

<template>
  <section class="external">
    <article class="connection-card">
      <h2>连接外部 Agent</h2>
      <p>复制连接说明并发给你的 Agent。一次接入即可操作 Dramlo、读取创作 Skill，并发现和导入 ComfyUI 工作流。</p>
      <div class="buttons">
        <button type="button" class="primary" :aria-expanded="showSetup" @click="copySetup">复制连接说明给 Agent</button>
        <button type="button" @click="download('Dramlo-Agent-接入.md', handoff)">下载接入说明</button>
        <button v-if="mcpCommand && mcpWorkspace" type="button" :aria-expanded="showConfig" @click="copyConfig">复制 Dramlo MCP 配置</button>
      </div>
      <p v-if="copyStatus" class="copy-status" role="status">{{ copyStatus }}</p>
      <p v-if="showConfig && !launcherToken">请在客户端另外设置与服务一致的 DRAMLO_MCP_BRIDGE_TOKEN。</p>
      <pre v-if="showConfig">{{ mcpConfig }}</pre>
      <pre v-if="showSetup">{{ handoff }}</pre>
    </article>
    <div v-if="!intro" class="buttons related"><RouterLink to="/settings/comfyui">ComfyUI 设置</RouterLink><RouterLink to="/creative-skills">创作 Skill 库</RouterLink></div>
  </section>
</template>

<style scoped>
.external { display: grid; gap: 14px; max-width: 840px; min-width: 0; }
article {
  min-width: 0;
  border: 1px solid var(--color-border);
  border-radius: 12px;
  background: var(--nb-panel-sheen), var(--glass-panel);
  box-shadow: var(--glass-elevation);
  padding: clamp(14px, 3vw, 26px);
}
.connection-card { border-color: var(--color-border-strong); }
h2 { margin: 0 0 8px; font-size: 19px; }
p { margin: 0 0 16px; color: var(--color-text-2); font-size: 14px; line-height: 1.65; }
.buttons { display: flex; flex-wrap: wrap; gap: 10px; }
button, .related a {
  display: inline-flex;
  align-items: center;
  min-height: 38px;
  padding: 6px 12px;
  border: 1px solid var(--color-border-strong);
  border-radius: 8px;
  background: var(--color-fill);
  color: var(--color-text-1);
  text-decoration: none;
  cursor: pointer;
}
button:hover, .related a:hover { background: var(--color-fill-hover); }
button.primary {
  border-color: transparent;
  background: var(--color-accent);
  color: var(--color-on-accent);
  font-weight: 650;
}
button.primary:hover { background: var(--glass-primary-hover); }
button:focus-visible, .related a:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.copy-status { margin: 12px 0 0; }
pre {
  margin: 14px 0 0;
  padding: 14px;
  border-radius: 8px;
  background: var(--nb-bg-inset);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 28rem;
  overflow: auto;
}
</style>
