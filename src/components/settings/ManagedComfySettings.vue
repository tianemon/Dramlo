<script setup lang="ts">
import AppNumberInput from '@/components/common/AppNumberInput.vue'
import { confirmAction } from '@/composables/useAppConfirmation'
import AppSelect from '@/components/common/AppSelect.vue'
import { onMounted, ref } from 'vue'
import { useSettingsStore } from '@/stores'

type Target = { root?: string; port: number }
type Browse = { label: string; source: { instanceId: string } | { target: Target } }
interface Installation { root: string; suggestedPort: number; canStart: boolean; online?: boolean; target: Target }
interface Instance { id: string; name: string; root?: string; port: number; online: boolean; managed: boolean; portConflict?: boolean }
interface Workflow {
  name: string
  source: 'disk' | 'server'
  folder?: number
  relativePath: string
  format?: 'ui' | 'api'
  configurationStatus?: 'configured' | 'available' | 'unknown'
  profiles?: Array<{ profileId: string; name: string }>
}
interface Catalog { installations: Installation[]; running: Array<{ port: number; address: string; root?: string; target: Target }> }

const settings = useSettingsStore()
const found = ref<Catalog>()
const rows = ref<Instance[]>([])
const root = ref('')
const port = ref<number | undefined>()
const active = ref<Browse>()
interface WorkflowDetails {
  weights: { nodeId: string; type: string; field: string; filename: string }[]
  candidateSlots: { nodeId: string; role: string; field: string }[]
}
const detail = ref<WorkflowDetails & { flow: Workflow }>()
const flows = ref<Workflow[]>([])
const unlinkedCount = ref(0)
const kind = ref<'image' | 'video' | ''>('')
const busy = ref(false)
const message = ref('')
const problem = ref(false)

async function call<T>(operation: string, input: object = {}): Promise<T> {
  const response = await fetch('/api/dramlo/comfy-instances', {
    method: 'POST', headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'studio_comfy_' + operation, input })
  })
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || '无法连接工作流管理服务')
  return result
}

async function run(work: () => Promise<void>) {
  if (busy.value) return
  busy.value = true
  message.value = ''
  problem.value = false
  try { await work() }
  catch (error) {
    problem.value = true
    message.value = error instanceof Error ? error.message : String(error)
  }
  finally { busy.value = false }
}

async function refresh() {
  rows.value = (await call<{ instances: Instance[] }>('instances')).instances
}

function submitDiscovery(event: SubmitEvent) {
  const input = (event.currentTarget as HTMLFormElement).querySelector<HTMLInputElement>('input[name=port]')
  if (input && !input.validity.valid) {
    problem.value = true
    message.value = '请输入 1 至 65535 的整数端口，或留空自动检测。'
    input.focus()
    return
  }
  return discover()
}

function discover() {
  return run(async () => {
    found.value = await call<Catalog>('discover', {
      roots: root.value.trim() ? [root.value.trim()] : undefined,
      ports: port.value ? [port.value] : undefined,
    })
    await refresh()
  })
}

function add(path: string | undefined, preferredPort?: number) {
  return run(async () => {
    await call('register', { root: path || undefined, port: preferredPort || undefined, confirmed: true })
    await refresh()
    message.value = '实例已保存到管理列表。'
  })
}

async function load(selection: Browse) {
  active.value = selection
  flows.value = []
  detail.value = undefined
  unlinkedCount.value = 0
  const result = await call<{ items: Workflow[]; problems: string[]; unlinkedProfiles?: unknown[] }>('workflows', selection.source)
  flows.value = result.items
  unlinkedCount.value = result.unlinkedProfiles?.length ?? 0
  if (result.problems.length) message.value = result.problems.join('。')
}

function browse(target: Target, label: string) {
  return run(() => load({ label, source: { target } }))
}

function readWorkflow(flow: Workflow) {
  return run(async () => {
    const selection = active.value
    if (!selection) return
    detail.value = undefined
    const result = await call<WorkflowDetails>('workflow_read', {
      ...selection.source, source: flow.source, folder: flow.folder, relativePath: flow.relativePath
    })
    detail.value = { flow, ...result }
  })
}

function lifecycle(instance: Instance, action: 'start' | 'stop') {
  return run(async () => {
    const result = await call<{ starting?: boolean }>(action, { instanceId: instance.id, confirmed: true })
    await refresh()
    message.value = action === 'stop' ? '已停止此实例。' : result.starting ? '已发起启动，可刷新查看状态。' : '实例已在线。'
  })
}

async function remove(instance: Instance) {
  if (!await confirmAction('移除「' + instance.name + '」的管理登记？已导入的工作流会保留。')) return
  return run(async () => {
    await call('remove', { instanceId: instance.id, confirmed: true })
    if (active.value && 'instanceId' in active.value.source && active.value.source.instanceId === instance.id) {
      active.value = undefined; flows.value = []; detail.value = undefined
    }
    await refresh()
    message.value = '已移除实例登记。'
  })
}

function importWorkflow(flow: Workflow) {
  return run(async () => {
    const selection = active.value
    if (!selection) return
    const result = await call<{ profileId: string; profileName: string; missingMapping: string[]; mappingIssue?: string; reused: boolean; updated: boolean }>('import', {
      ...selection.source, source: flow.source, folder: flow.folder, relativePath: flow.relativePath, kind: kind.value || undefined
    })
    await settings.reloadFromDisk()
    flow.configurationStatus = 'configured'
    flow.profiles = [
      ...(flow.profiles ?? []).filter(profile => profile.profileId !== result.profileId),
      { profileId: result.profileId, name: result.profileName }
    ]
    message.value = result.mappingIssue || result.missingMapping.length
      ? '已导入。' + (result.mappingIssue || '请补齐参数映射：' + result.missingMapping.join('、'))
      : result.updated ? '已更新原工作流配置。' : result.reused ? '此工作流已导入。' : '已加入对应的图像或视频接入，可在生成节点中选择。'
  })
}

onMounted(discover)
</script>

<template>
  <section class="comfy-managed">
    <h2>本机 ComfyUI</h2>
    <form class="row" novalidate @submit.prevent="submitDiscovery">
      <label class="path-label">安装目录<input v-model="root" placeholder="可留空，扫描常用目录" /></label>
      <label>端口<AppNumberInput v-model="port" name="port" min="1" max="65535" placeholder="自动" /></label>
      <button :disabled="busy">扫描本机</button>
      <button type="button" :disabled="busy || (!root.trim() && !port)" @click="add(root.trim() || undefined, port)">保存指定实例</button>
    </form>
    <section v-if="found?.installations.length || found?.running.length" class="panel">
      <h3>发现的实例</h3>
      <div v-for="candidate in found.installations" :key="candidate.root" class="row result">
        <div class="description"><strong>{{ candidate.root }}</strong><small>建议端口 {{ candidate.suggestedPort }} · {{ candidate.online ? '已在线' : candidate.canStart ? '找到实例 Python' : '未找到实例 Python' }}</small></div>
        <div class="row">
          <button type="button" :disabled="busy" @click="browse(candidate.target, candidate.root)">查看工作流</button>
          <button type="button" :disabled="busy" @click="add(candidate.root, candidate.target.port)">保存实例</button>
        </div>
      </div>
      <div v-for="candidate in found.running.filter(row => !found?.installations.some(item => item.root === row.root))" :key="candidate.address" class="row result">
        <div class="description"><strong>{{ candidate.address }}</strong><small>已在线 · 可接入现有服务</small></div>
        <div class="row">
          <button type="button" :disabled="busy" @click="browse(candidate.target, candidate.address)">查看工作流</button>
          <button type="button" :disabled="busy" @click="add(undefined, candidate.port)">保存实例</button>
        </div>
      </div>
    </section>
    <section class="panel">
      <div class="row result"><h3>已保存实例</h3><button type="button" :disabled="busy" @click="run(refresh)">刷新状态</button></div>
      <p v-if="!rows.length">长期管理或托管启停时保存实例。</p>
      <div v-for="instance in rows" :key="instance.id" class="row result">
        <div class="description"><strong>{{ instance.name }}</strong><small>{{ instance.port }} · {{ instance.portConflict ? '端口属于另一实例' : instance.online ? '在线' : '离线' }}</small></div>
        <button v-if="!instance.online && !instance.portConflict && instance.root" type="button" :disabled="busy" @click="lifecycle(instance, 'start')">启动</button>
        <button v-if="instance.online && instance.managed" type="button" :disabled="busy" @click="lifecycle(instance, 'stop')">停止</button>
        <button type="button" :disabled="busy" @click="run(() => load({ label: instance.name, source: { instanceId: instance.id } }))">工作流</button>
        <button type="button" :disabled="busy" @click="remove(instance)">移除登记</button>
      </div>
    </section>
    <section v-if="active" class="panel">
      <div class="row result">
        <h3>{{ active.label }} 的工作流</h3>
        <AppSelect v-model="kind" aria-label="导入用途"><option value="">自动识别用途</option><option value="image">图像生成</option><option value="video">视频生成</option></AppSelect>
      </div>
      <p v-if="!flows.length">未发现工作流。</p>
      <p v-if="unlinkedCount">此端点有 {{ unlinkedCount }} 个接入没有记录工作流来源，导入前可到模型列表核对。</p>
      <div v-for="flow in flows" :key="flow.source + ':' + flow.folder + ':' + flow.relativePath" class="row result">
        <div class="description">
          <strong>{{ flow.name }}</strong>
          <small>{{ flow.source === 'server' ? '在线' : '磁盘' }} · {{ flow.relativePath }}</small>
          <small v-if="flow.configurationStatus === 'configured'">已配置 · {{ flow.profiles?.map(item => item.name).join('、') }}</small>
          <small v-else-if="flow.configurationStatus === 'available'">可导入</small>
          <small v-else-if="flow.configurationStatus === 'unknown'">配置状态无法核对</small>
        </div>
        <div class="row">
          <button type="button" :disabled="busy" @click="readWorkflow(flow)">查看槽位</button>
          <button type="button" :disabled="busy" @click="importWorkflow(flow)">导入</button>
        </div>
      </div>
    </section>
    <section v-if="detail" class="panel">
      <div class="row result">
        <h3>{{ detail.flow.name }} · 槽位</h3>
        <button type="button" @click="detail = undefined">收起</button>
      </div>
      <p v-if="!detail.weights.length && !detail.candidateSlots.length">未识别到模型权重或可映射输入。</p>
      <ul v-if="detail.weights.length" aria-label="模型权重">
        <li v-for="weight in detail.weights" :key="weight.nodeId + ':' + weight.field">
          {{ weight.type }} · {{ weight.nodeId }}.{{ weight.field }} · {{ weight.filename }}
        </li>
      </ul>
      <ul v-if="detail.candidateSlots.length" aria-label="可映射输入">
        <li v-for="(slot, index) in detail.candidateSlots" :key="index">{{ slot.role }} · {{ slot.nodeId }}.{{ slot.field }}</li>
      </ul>
    </section>
    <p v-if="busy" role="status">正在处理 ComfyUI 请求…</p>
    <p v-if="message" :role="problem ? 'alert' : 'status'">{{ message }}</p>
  </section>
</template>

<style scoped>
.comfy-managed, .panel { display: grid; gap: 12px; }
li { overflow-wrap: anywhere; }
h2, h3, p { margin: 0; }
h3 { font-size: 14px; }
.panel { border: 1px solid var(--color-border-strong); border-radius: var(--radius-lg); padding: 12px; background: var(--nb-panel-sheen), var(--glass-panel); box-shadow: var(--glass-highlight); }
.row { display: flex; flex-wrap: wrap; align-items: end; gap: 8px; }
.result { align-items: center; justify-content: space-between; }
label, .description { display: grid; gap: 4px; min-width: 0; }
.path-label, .description { flex: 1; overflow-wrap: anywhere; }
.app-number { width: 78px; }
small, p { color: var(--color-text-3); }
button, input, .app-number, .app-select { padding: 7px 10px; color: inherit; background: var(--color-bg-2); border: 1px solid var(--color-border); border-radius: 6px; }
button { cursor: pointer; }
button:disabled { opacity: .55; cursor: default; }
</style>
