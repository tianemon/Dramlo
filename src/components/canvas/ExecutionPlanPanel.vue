<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import AppSelect from '@/components/common/AppSelect.vue'
import { computed, onUnmounted, ref, watch } from 'vue'

const props = defineProps<{ projectId: string }>()
type Progress = { state: string; assetId?: string; reason?: { error?: string; status?: string } }
type Plan = { id: string; name: string; version: number; state: string; pauseRequested: boolean; progress: Record<string, Progress> }
type Step = {
  id: string; kind: string; nodeId?: string; label?: string; state: string; reason?: { error?: string }
  requires?: { slot: string; source?: string; state: string }[]
  execution?: { profileId?: string }
}
type Snapshot = { updatedAt: number; planVersion: number; executionRevision: string; execution: Step[] }
const open = ref(false)
const dialog = ref<HTMLDialogElement | null>(null)
const busy = ref(false)
const error = ref('')
const selected = ref('')
const plans = ref<Plan[]>([])
const graph = ref<Snapshot>()
const plan = computed(() => plans.value.find(item => item.id === selected.value))
const blocked = computed(() => graph.value?.execution.some(item => item.state === 'blocked' && !plan.value?.progress[item.id]))
const status: Record<string, string> = {
  draft: '草稿', running: '执行中', paused: '已暂停', interrupted: '服务已中断', succeeded: '已完成',
  blocked: '阻塞', unknown: '结果未知', waiting: '等待上游', ready: '就绪', bound: '已绑定', missing: '缺输入',
  'missing-file': '素材文件缺失'
}
const caption: Record<string, string> = { image: '生图', video: '视频', tail: '提取尾帧', export: '导出' }
let epoch = 0

async function call<T>(name: string, data: object = {}): Promise<T> {
  const response = await fetch('/api/dramlo/execution-plan', {
    method: 'POST',
    headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, input: { ...data, projectId: props.projectId } })
  })
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || '执行计划操作失败')
  return result
}

async function refresh() {
  const generation = ++epoch
  try {
    const result = await call<{ plans: Plan[] }>('studio_current_plan', { includeCompleted: true })
    if (generation !== epoch) return
    plans.value = result.plans
    if (!result.plans.some(item => item.id === selected.value)) selected.value = result.plans[0]?.id ?? ''
    const id = selected.value
    const next = id ? await call<Snapshot>('studio_canvas_graph', { planId: id }) : undefined
    if (generation === epoch && id === selected.value) graph.value = next
  } catch (cause) {
    if (generation === epoch) error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

async function takeAction(pause = false, retry = false) {
  if (!plan.value || !graph.value || busy.value) return
  const projectId = props.projectId
  const id = selected.value
  const snapshot = graph.value
  if (!pause && !await confirmAction(retry ? '确认重做当前已知失败的步骤？' : `确认执行「${plan.value.name}」？Dramlo 将依次运行尚未完成的步骤。`)) return
  if (!open.value || busy.value || props.projectId !== projectId || selected.value !== id) return
  if (graph.value?.planVersion !== snapshot.planVersion || graph.value.executionRevision !== snapshot.executionRevision) {
    error.value = '执行计划已变化，请核对后重新确认。'
    return
  }
  error.value = ''
  busy.value = true
  try {
    await call(pause ? 'studio_pause_plan' : 'studio_run_plan', pause ? { planId: id } : {
      planId: id, expectedPlanVersion: snapshot.planVersion, expectedUpdatedAt: snapshot.updatedAt,
      expectedExecutionRevision: snapshot.executionRevision, confirmed: true, ...(retry ? { retryFailed: true } : {})
    })
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    busy.value = false
    await refresh()
  }
}

function openPlan(planId?: string) {
  if (planId) selected.value = planId
  if (!dialog.value) return
  if (dialog.value.open) {
    void refresh()
    return
  }
  dialog.value.showModal()
  open.value = true
  error.value = ''
  void refresh()
}

defineExpose({ openPlan })

function closeOutside(event: MouseEvent) {
  const target = dialog.value
  if (!target) return
  const box = target.getBoundingClientRect()
  if (event.clientX < box.left || event.clientX > box.right ||
      event.clientY < box.top || event.clientY > box.bottom) target.close()
}

watch(() => props.projectId, () => {
  epoch++
  selected.value = ''
  graph.value = undefined
  plans.value = []
  error.value = ''
  if (open.value) void refresh()
})
function choose() {
  graph.value = undefined
  void refresh()
}
const timer = window.setInterval(() => {
  if (open.value && !busy.value && (plan.value?.state === 'running' || plan.value?.pauseRequested)) void refresh()
}, 1600)
onUnmounted(() => { epoch++; window.clearInterval(timer) })
</script>

<template>
  <button class="plan-trigger" type="button" aria-haspopup="dialog" @click="openPlan()">执行计划</button>
  <Teleport to="body">
    <dialog ref="dialog" class="plan-dialog" aria-label="执行计划" @close="open = false" @click.self="closeOutside">
        <header><h2>执行计划</h2><button type="button" autofocus @click="dialog?.close()">关闭</button></header>
        <p v-if="error" role="alert">{{ error }}</p>
        <p v-if="!plans.length && !error">这个项目还没有执行计划。可让 Agent 保存计划，再到这里核对并启动。</p>
        <template v-if="plans.length">
          <label>计划 <AppSelect v-model="selected" @change="choose">
            <option v-for="item in plans" :key="item.id" :value="item.id">{{ item.name }}</option>
          </AppSelect></label>
          <p v-if="plan">{{ plan.pauseRequested && plan.state === 'running' ? '当前步骤结束后暂停' : status[plan.state] || plan.state }}</p>
          <ol v-if="graph" class="plan-steps">
            <li v-for="step in graph.execution" :key="step.id">
              <strong :title="step.nodeId">{{ caption[step.kind] }} · {{ step.label || step.id }}</strong>
              <span>{{ status[plan?.progress[step.id]?.state || step.state] }}</span>
              <small v-if="step.execution?.profileId">工作流 {{ step.execution.profileId }}</small>
              <small v-if="plan?.progress[step.id]?.assetId">素材 {{ plan.progress[step.id]?.assetId }}</small>
              <small v-if="plan?.progress[step.id]?.reason?.error || step.reason?.error">{{ plan?.progress[step.id]?.reason?.error || step.reason?.error }}</small>
              <ul v-if="step.requires?.length">
                <li v-for="(requirement, index) in step.requires" :key="index">
                  {{ requirement.slot }}{{ requirement.source ? ' · ' + requirement.source : '' }} · {{ status[requirement.state] }}
                </li>
              </ul>
            </li>
          </ol>
          <footer>
            <button type="button" :disabled="busy" @click="refresh()">刷新</button>
            <button v-if="plan?.state === 'running'" type="button" :disabled="busy || plan.pauseRequested" @click="takeAction(true)">暂停后续</button>
            <button v-else-if="plan?.state !== 'succeeded'" type="button" :disabled="busy || !graph || blocked" @click="takeAction()">{{ plan?.state === 'draft' ? '启动计划' : '继续计划' }}</button>
            <button v-if="plan?.state === 'blocked' && Object.values(plan.progress).some(item => ['failed', 'interrupted', 'discarded'].includes(item.reason?.status ?? ''))"
              type="button" :disabled="busy || !graph || blocked" @click="takeAction(false, true)">重做失败步骤</button>
          </footer>
        </template>
    </dialog>
  </Teleport>
</template>

<style scoped>
.plan-trigger { color: inherit; background: var(--color-fill); border: 1px solid transparent; border-radius: 6px; padding: 5px 10px; cursor: pointer; }
.plan-trigger:hover:not(:disabled) { background: var(--control-hover); }
.plan-trigger:active:not(:disabled) { background: var(--control-pressed); }
.plan-dialog::backdrop { background: #0008; }
.plan-dialog { width: min(600px, 90vw); max-height: 80vh; overflow: auto; border: 1px solid var(--color-border); border-radius: 12px; background: var(--glass-panel); color: var(--color-text-1); box-shadow: var(--shadow-lg); padding: 20px; }
header, footer { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
h2 { margin: 0 0 8px; font-size: 17px; }
.plan-steps { padding-inline-start: 22px; }
.plan-steps > li { margin-bottom: 12px; }
.plan-steps span { margin-inline-start: 8px; }
small { display: block; }
ul { padding-inline-start: 18px; }
button:disabled { cursor: default; opacity: 0.5; }
</style>
