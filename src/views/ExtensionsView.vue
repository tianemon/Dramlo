<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import { ref, watch } from 'vue'
import { useProjectStore } from '@/stores'
import { extensionRequest, type CanvasExtensionType } from '@/services/canvas/extensions'

const project = useProjectStore()
const items = ref<CanvasExtensionType[]>([])
const manifest = ref<Record<string, unknown>>()
const preview = ref<CanvasExtensionType[]>([])
const packageData = ref('')
const providerText = ref('{"id":"my-mcp","command":"","args":[],"tools":[]}')
const configureOpen = ref(false)
const providers = ref<{ id: string; command: string; tools: string[] }[]>([])
async function reloadProviders() {
  const result = await extensionRequest<{ items: typeof providers.value }>('list_mcp_providers', {})
  providers.value = result.items
}
const providerConfirmed = ref(false)
const error = ref('')
const status = ref('')
const busy = ref(false)
let loadId = 0

async function reload() {
  const request = ++loadId
  const result = await extensionRequest<{ items: CanvasExtensionType[] }>('studio_extension_library', { projectId: project.current?.id })
  if (request === loadId) items.value = result.items
}

async function action(run: () => Promise<void>) {
  busy.value = true
  error.value = ''
  status.value = ''
  try { await run() }
  catch (failure) { error.value = failure instanceof Error ? failure.message : String(failure) }
  finally { busy.value = false }
}

watch(() => project.current?.id, () => {
  void reload().catch(failure => { error.value = String(failure) })
}, { immediate: true })

async function fileSelected(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  manifest.value = undefined
  preview.value = []
  packageData.value = ''
  if (!file) return
  await action(async () => {
    if (file.name.toLowerCase().endsWith('.zip')) {
      if (file.size > 128 * 1024) throw new Error('扩展 ZIP 不得超过 128 KB')
      const bytes = new Uint8Array(await file.arrayBuffer())
      const data = btoa(Array.from(bytes, value => String.fromCharCode(value)).join(''))
      preview.value = (await extensionRequest<{ items: CanvasExtensionType[] }>('studio_extension_package_preview', { packageBase64: data })).items
      packageData.value = data
      return
    }
    if (file.size > 64 * 1024) throw new Error('扩展清单不得超过 64 KB')
    const data: Record<string, unknown> = JSON.parse(await file.text())
    preview.value = [await extensionRequest<CanvasExtensionType>('studio_extension_preview', { manifest: data })]
    manifest.value = data
  })
}

async function install() {
  await action(async () => {
    if (packageData.value) await extensionRequest('studio_extension_package_install', { packageBase64: packageData.value })
    else await extensionRequest('studio_extension_install', { manifest: manifest.value })
    manifest.value = undefined
    preview.value = []
    packageData.value = ''
    await reload()
    status.value = '已安装，可按项目启用'
  })
}

function toggle(item: CanvasExtensionType) {
  const projectId = project.current?.id
  if (!projectId) return
  void action(async () => {
    await extensionRequest('studio_extension_enable', {
      projectId, typeId: item.id, version: item.version, enabled: !item.enabled
    })
    await reload()
  })
}

function exportPackage(item: CanvasExtensionType) {
  void action(async () => {
    const result = await extensionRequest<{ packageBase64: string }>('studio_extension_package_export', {
      typeId: item.id, version: item.version
    })
    const bytes = Uint8Array.from(atob(result.packageBase64), value => value.charCodeAt(0))
    const url = URL.createObjectURL(new Blob([bytes], { type: 'application/zip' }))
    const link = document.createElement('a')
    link.href = url
    link.download = item.id + '.zip'
    link.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  })
}

function toggleProviders() {
  configureOpen.value = !configureOpen.value
  if (configureOpen.value) void action(reloadProviders)
}

async function removeProvider(providerId: string) {
  if (!await confirmAction('撤销 MCP 连接 ' + providerId + '？相关扩展将无法新发起调用。')) return
  void action(async () => {
    await extensionRequest('remove_mcp_provider', { providerId, confirmed: true })
    await reloadProviders()
  })
}

async function removeExtension(item: CanvasExtensionType) {
  if (!await confirmAction('卸载 ' + item.name + '？已保存节点和素材会保留。')) return
  void action(async () => {
    await extensionRequest('studio_extension_remove', { typeId: item.id, version: item.version, confirmed: true })
    await reload()
  })
}

function configureProvider() {
  void action(async () => {
    const provider = JSON.parse(providerText.value)
    await extensionRequest('configure_mcp_provider', { provider, confirmed: providerConfirmed.value })
    await reloadProviders()
    providerConfirmed.value = false
    status.value = '已登记 MCP 连接'
  })
}

function download(data: Record<string, unknown>, filename: string) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }))
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

function mcpTemplate() {
  download({ formatVersion: 1, id: 'example.mcp-tool', version: 1, name: '我的 MCP 工具',
    description: '填写工具用途及参数说明。', executor: 'studio.mcp', providerId: 'my-mcp', tool: '工具名' }, 'extension.json')
}

function template(comfy = false) {
  download(comfy ? { formatVersion: 1, id: 'example.comfy-video', version: 1, name: '我的 ComfyUI 视频工作流',
    description: '用已在 Dramlo 登记的工作流生成分镜视频。', executor: 'studio.comfy-video', profileId: '填写已导入的视频配置 ID' }
    : { formatVersion: 1, id: 'example.video-tail', version: 1, name: '我的尾帧节点',
      description: '从视频中提取最后一帧。', executor: 'studio.video-tail' }, 'extension.json')
}
</script>

<template>
  <main class="extensions">
    <header>
      <h1>扩展库</h1>
      <p>{{ project.current ? '当前项目 · ' + project.current.name : '打开项目后可启用自定义扩展' }}</p>
      <div class="actions">
        <label>预检 JSON 或 ZIP 扩展 <input type="file" accept=".json,.zip,application/json,application/zip" :disabled="busy" @change="fileSelected" /></label>
        <button type="button" @click="template()">下载尾帧清单模板</button>
        <button type="button" @click="template(true)">下载 ComfyUI 清单模板</button>
        <button type="button" @click="mcpTemplate">下载 MCP 清单模板</button>
        <button type="button" @click="toggleProviders">管理本机 MCP</button>
      </div>
      <p v-if="error" role="alert">{{ error }}</p>
      <p v-if="status" role="status">{{ status }}</p>
    </header>
    <article v-if="configureOpen">
      <ul v-if="providers.length">
        <li v-for="provider in providers" :key="provider.id">
          <strong>{{ provider.id }}</strong> · {{ provider.tools.join('、') }}
          <p>{{ provider.command }}</p>
          <button type="button" :disabled="busy" @click="removeProvider(provider.id)">撤销连接</button>
        </li>
      </ul>
      <label>连接配置 JSON <textarea v-model="providerText" rows="5" /></label>
      <p>填写本机可执行文件的绝对路径、参数和允许调用的工具名。进程使用当前用户的系统权限。</p>
      <label><input v-model="providerConfirmed" type="checkbox" /> 我确认信任此可执行文件及参数</label>
      <button type="button" :disabled="busy || !providerConfirmed" @click="configureProvider">保存连接</button>
    </article>
    <article v-if="preview.length">
      <section v-for="entry in preview" :key="entry.id + '@' + entry.version">
        <strong>{{ entry.name }}</strong>
        <p>{{ entry.description }}</p>
        <p>{{ entry.id }} · 版本 {{ entry.version }} · {{ entry.executor }}</p>
      </section>
      <button type="button" :disabled="busy" @click="install">安装扩展</button>
    </article>
    <article v-for="item in items" :key="item.id + '@' + item.version">
      <strong>{{ item.name }}</strong>
      <small>{{ item.source === 'built-in' ? '内置' : item.source === 'imported' ? '外部导入' : '自定义' }} · 版本 {{ item.version }} · {{ item.enabled ? '已启用' : '未启用' }}</small>
      <p>{{ item.description }}</p>
      <p>{{ item.id }}</p>
      <div v-if="item.source !== 'built-in'" class="actions">
        <button type="button" :disabled="busy || !project.current" @click="toggle(item)">{{ item.enabled ? '停用' : '启用' }}</button>
        <button v-if="item.manifest" type="button" @click="download(item.manifest, item.id + '.json')">导出清单</button>
        <button type="button" :disabled="busy" @click="exportPackage(item)">导出 ZIP</button>
        <button type="button" :disabled="busy" @click="removeExtension(item)">卸载</button>
      </div>
    </article>
  </main>
</template>

<style scoped>
.extensions { box-sizing: border-box; height: 100%; overflow: auto; max-width: 960px; margin: 0 auto; padding: 24px; }
h1 { margin: 0 0 8px; }
p { margin: 8px 0; overflow-wrap: anywhere; }
article { margin-top: 14px; padding: 16px; border: 1px solid var(--color-border); border-radius: 10px; }
small { margin-inline-start: 12px; color: var(--color-text-3); }
textarea { display: block; width: 100%; margin: 8px 0; }
.actions { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
button { border: 1px solid var(--color-border); border-radius: 6px; color: inherit; background: transparent; padding: 6px 12px; cursor: pointer; }
button:disabled { opacity: .5; cursor: default; }
</style>
