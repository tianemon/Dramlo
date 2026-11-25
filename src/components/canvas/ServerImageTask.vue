<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import { computed, onUnmounted, ref, watch } from 'vue'
import { useAssetsStore, useProjectStore } from '@/stores'

const props = defineProps<{
  targetType: 'scene' | 'shot-image'
  targetId: string
  savedPrompt?: string
  startOpen?: boolean
  suggestedReferenceIds?: string[]
}>()
const project = useProjectStore()
const assets = useAssetsStore()
const opened = ref(Boolean(props.startOpen))
const prompt = ref('')
const references = ref('')
const profileId = ref('')
const outputNodeId = ref('')
const reviewNote = ref('')
const feedback = ref('')
const busy = ref(false)
type Job = { requestId: string; status: string; prompt: string; profileId: string; referenceAssetIds: string[];
  promptSource?: string; referenceAssignments?: Array<{ nodeId?: string; assetId?: string; fallback?: boolean }>;
  assetId?: string; selected?: boolean; error?: string; warnings?: string[];
  nextAction?: { message: string };
  aspect?: { projectAspectRatio?: string; size?: string; workflowSelectors?: { nodeId: string; value: string }[] } }
const job = ref<Job>()
const active = computed(() => ['starting', 'submitting', 'waiting'].includes(job.value?.status ?? ''))
const promptSourceLabels: Record<string, string> = {
  request: '本次填写', imagePrompt: '镜头生图词', visualPrompt: '分镜画面描述', platePrompt: '场景生图词'
}
const storageKey = computed(() => 'studio-image-job:' + [project.current?.id, props.targetType, props.targetId].join(':'))
let refreshRunning = false
let actionVersion = 0
let disposed = false

async function call(name: string, data: object): Promise<Job | undefined> {
  const response = await fetch('/api/dramlo/image-queue', {
    method: 'POST',
    headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, input: data })
  })
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || '服务端生图操作失败')
  return result.job ?? undefined
}

async function refresh() {
  const id = project.current?.id
  const key = storageKey.value
  const version = actionVersion
  if (!id || refreshRunning || busy.value) return
  refreshRunning = true
  try {
    const next = await call('studio_current_image_job', { projectId: id, targetType: props.targetType, targetId: props.targetId })
    if (disposed || version !== actionVersion || key !== storageKey.value) return
    const previous = job.value?.status
    job.value = next
    if (next?.status === 'succeeded' && previous !== next.status) await assets.refresh()
  } catch (error) {
    if (!disposed && version === actionVersion && key === storageKey.value) {
      feedback.value = error instanceof Error ? error.message : String(error)
    }
  } finally { refreshRunning = false }
}

watch(storageKey, () => {
  actionVersion++
  job.value = undefined
  prompt.value = ''
  references.value = ''
  profileId.value = ''
  outputNodeId.value = ''
  feedback.value = ''
  reviewNote.value = ''
  if (opened.value) void refresh()
})
watch(() => job.value?.status === 'unknown' ? job.value.requestId : undefined, () => {
  reviewNote.value = ''
})
watch(opened, value => { if (value) void refresh() }, { immediate: true })
const timer = setInterval(() => { if (opened.value) void refresh() }, 1200)
onUnmounted(() => {
  disposed = true
  clearInterval(timer)
})

async function execute(action: 'prepare' | 'confirm' | 'resume' | 'resolve_unknown') {
  const id = project.current?.id
  if (!id || busy.value) return
  const key = storageKey.value
  const requestId = action === 'prepare' ? crypto.randomUUID() : job.value?.requestId
  if (!requestId) return
  const reason = reviewNote.value.trim()
  if (action === 'resolve_unknown') {
    if (job.value?.status !== 'unknown' || !reason) return
    if (!await confirmAction(`确认已核对生图请求 ${requestId} 的供应商记录，原请求已停止且没有可取回的结果？`)) return
    if (disposed || key !== storageKey.value || busy.value) return
    if (job.value?.requestId !== requestId || job.value.status !== 'unknown') {
      feedback.value = '当前生图请求已变化，请核对后重新确认。'
      return
    }
  }
  const version = ++actionVersion
  busy.value = true
  feedback.value = ''
  try {
    if (action === 'prepare' && project.dirty) await project.save()
    if (key !== storageKey.value || project.current?.id !== id || (action === 'prepare' && project.dirty)) {
      throw new Error('项目已切换或存在未保存修改')
    }
    const values = action === 'prepare' ? {
      expectedUpdatedAt: project.current.updatedAt, targetType: props.targetType, targetId: props.targetId,
      ...(profileId.value.trim() ? { profileId: profileId.value.trim() } : {}),
      ...(prompt.value.trim() ? { prompt: prompt.value.trim() } : {}),
      ...(outputNodeId.value.trim() ? { outputNodeId: outputNodeId.value.trim() } : {}),
      referenceAssetIds: references.value.split(/[\s,，]+/).filter(Boolean)
    } : { confirmed: true, ...(action === 'resolve_unknown' ? { reason } : {}) }
    const next = await call('studio_' + action + '_image', { projectId: id, requestId, ...values })
    if (!disposed && version === actionVersion && key === storageKey.value) {
      job.value = next
      if (action === 'resolve_unknown') {
        reviewNote.value = ''
        feedback.value = '已关闭原请求，可以重新预检。'
      }
    }
  } catch (error) {
    if (!disposed && version === actionVersion && key === storageKey.value) {
      feedback.value = error instanceof Error ? error.message : String(error)
    }
  } finally { busy.value = false }
}
</script>

<template>
  <details :open="opened" @toggle="opened = ($event.target as HTMLDetailsElement).open">
    <summary>服务端生图</summary>
    <div v-if="opened" class="image-task">
      <label>图像配置 ID（可选） <input v-model="profileId" placeholder="使用当前图像配置" /></label>
      <label>输出节点 ID（可选） <input v-model="outputNodeId" placeholder="使用工作流返回的第一张图" /></label>
      <label>生图提示词 <textarea v-model="prompt" rows="3" :placeholder="savedPrompt ? '留空使用节点已存提示词' : '填写实际提交的提示词'" /></label>
      <label>已配置槽位的参考图 ID <input v-model="references" placeholder="按槽位顺序，用逗号分隔" /></label>
      <div class="actions">
        <button v-if="suggestedReferenceIds?.length" type="button" :disabled="busy || active"
          @click="references = suggestedReferenceIds.join(', ')">填入镜头参考图</button>
        <button type="button" :disabled="busy || active || job?.status === 'unknown' || job?.status === 'paused'" @click="execute('prepare')">预检</button>
        <button v-if="job?.status === 'prepared'" type="button" :disabled="busy" @click="execute('confirm')">确认生成</button>
        <button v-if="job?.status === 'paused'" type="button" :disabled="busy" @click="execute('resume')">继续查询原任务</button>
      </div>
      <template v-if="job">
        <p role="status">{{ ({ discarded: '原请求已关闭', prepared: '待确认', starting: '准备中', submitting: '提交中', waiting: '生成中', succeeded: '已保存', paused: '等待暂停', unknown: '提交结果未知', failed: '失败' } as Record<string, string>)[job.status] }}</p>
        <p v-if="job.status === 'prepared'">配置 {{ job.profileId }} · 参考图 {{ job.referenceAssetIds.length }} 张</p>
        <p v-if="job.status === 'prepared' && job.promptSource">提示词来源 · {{ promptSourceLabels[job.promptSource] || job.promptSource }}</p>
        <ol v-if="job.status === 'prepared' && job.referenceAssetIds.length" aria-label="预检参考图顺序">
          <li v-for="(id, index) in job.referenceAssetIds" :key="index">
            {{ id }}<template v-if="job.referenceAssignments?.[index]?.nodeId"> · 槽 {{ job.referenceAssignments[index].nodeId }}</template>
            <template v-if="job.referenceAssignments?.[index]?.fallback"> · 补位图</template>
          </li>
        </ol>
        <p v-if="job.status === 'prepared' && job.aspect">
          项目画幅 {{ job.aspect.projectAspectRatio || '未设置' }} · 输出尺寸
          {{ job.aspect.size || job.aspect.workflowSelectors?.map(selector => selector.value).join('、') || '未识别' }}
        </p>
        <ul v-if="job.warnings?.length" aria-label="预检提醒">
          <li v-for="warning in job.warnings" :key="warning">{{ warning }}</li>
        </ul>
        <p v-if="job.status === 'prepared'" class="plan-prompt">{{ job.prompt }}</p>
        <p v-if="job.assetId">{{ job.selected ? '已绑定到原节点' : '已存入项目素材库' }} · {{ job.assetId }}</p>
        <p v-if="job.error" role="alert">{{ job.error }}</p>
        <p v-if="job.nextAction">{{ job.nextAction.message }}</p>
        <template v-if="job.status === 'unknown'">
          <label>供应商核对结果 <input v-model="reviewNote" maxlength="500" /></label>
          <button type="button" :disabled="busy || !reviewNote.trim()" @click="execute('resolve_unknown')">确认核对并关闭原请求</button>
        </template>
      </template>
      <p v-if="feedback" role="alert">{{ feedback }}</p>
    </div>
  </details>
</template>

<style scoped>
summary { cursor: pointer; border-radius: var(--radius-sm); }
summary:hover { background: var(--control-hover); }
summary:active { background: var(--control-pressed); }
.image-task, label { display: grid; gap: 6px; }
.image-task { padding: 8px 0; }
.actions { display: flex; flex-wrap: wrap; gap: 8px; }
.image-task button { border-radius: var(--radius-sm); }
.image-task button:disabled { cursor: default; opacity: 0.5; }
.plan-prompt { max-height: 140px; overflow: auto; white-space: pre-wrap; }
input, textarea { width: 100%; }
</style>
