<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { isDesktop } from '@/services/desktop'
import { callWebVideoQueue } from '@/services/localWebStorage'
import { clearServerVideoActivity, setServerVideoActivity } from '@/services/canvas/jobs'
import { useToastStore } from '@/stores'

const props = defineProps<{ projectId: string }>()
const toast = useToastStore()
type Batch = {
  projectId: string
  requestId: string
  status: 'prepared' | 'running' | 'paused' | 'succeeded' | 'failed' | 'unknown' | 'discarded'
  pauseRequested?: boolean
  error?: string
  steps: Array<{
    shotId: string
    title: string
    profileId: string
    aspect: string
    take: { mode: string; firstFrameAssetId?: string; lastFrameAssetId?: string }
    prompt: string
    warnings?: string[]
    referenceAssignments?: Array<{ assetId?: string }>
    mediaInputs?: Array<{ role: string; nodeId?: string; assetId?: string; shotId?: string }>
    templateImages?: Array<{ nodeId: string; filename: string }>
    workload?: { videoSeconds?: number }
    bind?: { slot: 'video'; applied: boolean; reason: string; current: string | null; isCurrent: boolean }
    previousShotId?: string
    promptId?: string
    status: 'ready' | 'submitting' | 'waiting' | 'saved' | 'failed' | 'unknown' | 'discarded'
    error?: string
  }>
}
const frameIds = (step: Batch['steps'][number]) => [...new Set([
  ...(step.referenceAssignments ?? []).map(item => item.assetId),
  step.take.firstFrameAssetId, step.take.lastFrameAssetId
].filter(Boolean))]
const mediaRole: Record<string, string> = {
  reference: '参考图', fallback: '补位图', 'first-frame': '首帧', 'last-frame': '尾帧', 'previous-tail': '上一镜尾帧'
}
const savedLabel = (step: Batch['steps'][number]) => {
  if (!step.bind) return '已归档'
  if (step.bind.isCurrent) return '已归档，当前选用'
  if (step.bind.reason === 'shot_missing') return '已归档，原镜头已移除'
  if (step.bind.applied) return '已归档，镜头已另选版本'
  return '已归档为候选，可在镜头的历史版本中选用'
}
const batch = ref<Batch | null>(null)
const activityOwner = Symbol()
const candidateCount = computed(() => batch.value?.steps.filter(step =>
  step.status === 'saved' && step.bind && !step.bind.isCurrent && step.bind.reason !== 'shot_missing'
).length ?? 0)
const busy = ref(false)
const expanded = ref(false)
let timer: ReturnType<typeof setInterval> | undefined
let disposed = false
let refreshVersion = 0

const labels = {
  prepared: '待确认', running: '制作中', paused: '已暂停', succeeded: '制作完成', failed: '制作失败', unknown: '提交结果未知', discarded: '已撤销'
}

async function refresh() {
  if (busy.value || !isDesktop() || !props.projectId) return
  const projectId = props.projectId
  const version = ++refreshVersion
  try {
    const result = await callWebVideoQueue<{ batch: Batch | null }>('studio_current_video_batch', { projectId })
    if (!disposed && version === refreshVersion && projectId === props.projectId) {
      batch.value = result.batch
      const active = Object.fromEntries((result.batch?.status === 'running' ? result.batch.steps : [])
        .filter(step => ['ready', 'submitting', 'waiting'].includes(step.status))
        .map(step => [step.shotId, step.status === 'ready' ? 'pending' as const : 'running' as const]))
      setServerVideoActivity(projectId, activityOwner, active)
    }
  } catch {
    // A local project switch can invalidate an in-flight query.
  }
}

async function act(action: 'start' | 'pause' | 'stop-wait' | 'cancel' | 'continue') {
  const current = batch.value
  if (!current || busy.value) return
  const waiting = action === 'cancel' ? current.steps.find(step => step.status === 'waiting' && step.promptId) : undefined
  if (action === 'cancel' && !waiting?.promptId) return
  const expectedPromptId = waiting?.promptId
  if (action === 'start' && !await confirmAction('确认按当前计划提交 ' + current.steps.length + ' 个镜头？')) return
  if (action === 'continue' && !await confirmAction('继续此批剩余镜头或查询已有任务？')) return
  if (waiting && !await confirmAction(`在 ComfyUI 中停止「${waiting.title}」并暂停后续镜头？`)) return
  if (disposed || busy.value) return
  if (props.projectId !== current.projectId || batch.value?.requestId !== current.requestId ||
      (expectedPromptId && !batch.value.steps.some(step => step.status === 'waiting' && step.promptId === expectedPromptId))) {
    toast.warn('视频批次或当前镜头已变化，请核对后重新操作。')
    return
  }
  const key = { projectId: current.projectId, requestId: current.requestId }
  busy.value = true
  try {
    const operation = action === 'start' ? 'studio_confirm_video'
      : action === 'continue' ? 'studio_resume_video'
        : action === 'cancel' ? 'studio_cancel_video_shot'
          : action === 'stop-wait' ? 'studio_stop_video_wait' : 'studio_pause_video'
    const result = await callWebVideoQueue<{ cancelled?: boolean }>(operation, {
      ...key, confirmed: true, ...(expectedPromptId ? { expectedPromptId } : {})
    })
    if (action === 'cancel' && result.cancelled !== true) toast.warn('ComfyUI 未取消当前任务。请查看镜头状态。')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法更新制作任务')
  } finally {
    busy.value = false
    await refresh()
  }
}

watch(() => props.projectId, (_, previous) => {
  clearServerVideoActivity(previous, activityOwner)
  batch.value = null
  expanded.value = false
  void refresh()
})
onMounted(() => {
  void refresh()
  timer = setInterval(refresh, 1300)
})
onUnmounted(() => {
  disposed = true
  clearInterval(timer)
  clearServerVideoActivity(props.projectId, activityOwner)
})
</script>

<template>
  <section v-if="batch" class="batch" aria-label="视频制作批次">
    <div class="row">
      <button type="button" :aria-expanded="expanded" @click="expanded = !expanded">
        {{ labels[batch.status] }} · {{ batch.steps.filter((step) => step.status === 'saved').length }} / {{ batch.steps.length }}
        <template v-if="candidateCount"> · {{ candidateCount }} 个未选版本</template>
      </button>
      <button v-if="batch.status === 'prepared' && expanded" :disabled="busy" type="button" @click="act('start')">确认开始</button>
      <button v-if="batch.status === 'running'" :disabled="busy || batch.pauseRequested" type="button" @click="act('pause')">
        {{ batch.pauseRequested ? '当前镜头完成后暂停' : '暂停后续镜头' }}
      </button>
      <button v-if="batch.status === 'running' && batch.steps.some((step) => step.status === 'waiting' && step.promptId)" :disabled="busy" type="button" @click="act('cancel')">停止当前生成</button>
      <button v-if="batch.status === 'running'" :disabled="busy" type="button" title="已交给 ComfyUI 的任务可继续" @click="act('stop-wait')">停止本地等待</button>
      <button v-if="batch.status === 'paused' && !batch.steps.some((step) => step.status === 'unknown')" :disabled="busy" type="button" @click="act('continue')">继续</button>
    </div>
    <template v-if="expanded">
      <p v-if="batch.error" role="alert">{{ batch.error }}</p>
      <ul>
        <li v-for="step in batch.steps" :key="step.shotId">
          {{ step.title }} · {{ step.profileId }}
          <template v-if="batch.status === 'prepared'">
            <p>{{ step.aspect }} · {{ step.take.mode }}<template v-if="step.workload?.videoSeconds"> · {{ step.workload.videoSeconds }} 秒</template></p>
            <p>{{ step.prompt }}</p>
            <p v-for="warning in step.warnings" :key="warning" role="status">{{ warning }}</p>
            <template v-if="step.mediaInputs?.length">
              <p v-for="(input, index) in step.mediaInputs" :key="index">
                {{ mediaRole[input.role] || input.role }}{{ input.nodeId ? ' · 槽 ' + input.nodeId : '' }}
                · {{ input.assetId || (input.shotId ? '镜头 ' + input.shotId + ' 的已选成片' : '待上游就绪') }}
              </p>
            </template>
            <template v-else>
              <p v-if="frameIds(step).length">素材 {{ frameIds(step).join('、') }}</p>
              <p v-if="step.previousShotId">接上一镜 {{ step.previousShotId }} 的已选成片尾帧</p>
            </template>
            <p v-for="image in step.templateImages" :key="image.nodeId">工作流固定图 · 槽 {{ image.nodeId }} · {{ image.filename }}</p>
          </template>
          <span v-if="step.status === 'saved'"> · {{ savedLabel(step) }}</span>
          <span v-if="step.promptId"> · 任务 {{ step.promptId }}</span>
          <span v-if="step.error"> · {{ step.error }}</span>
        </li>
      </ul>
    </template>
  </section>
</template>

<style scoped>
.batch { padding: 6px 12px; border-bottom: 1px solid var(--nb-line-1); }
.row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
button { font: inherit; color: inherit; background: var(--color-fill); border: 1px solid transparent; border-radius: 5px; padding: 4px 8px; cursor: pointer; }
button:hover:not(:disabled) { background: var(--control-hover); }
button:active:not(:disabled) { background: var(--control-pressed); }
p, ul { margin: 8px 0; }
</style>
