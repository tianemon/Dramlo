<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import AppSelect from '@/components/common/AppSelect.vue'
import { computed, ref } from 'vue'
import { useProjectStore, useToastStore } from '@/stores'
import { approveExternalCapturePhase, setExternalCapturePlan, grabLastFrame } from '@/services/continuous/capture'

const props = defineProps<{ selectedNodeId?: string; webWorkspace?: boolean }>()
const emit = defineEmits<{ 'show-plan': [planId: string] }>()
const approvalMode = ref<'batch' | 'scene'>('batch')
const project = useProjectStore()
const toast = useToastStore()
const capture = computed(() => project.current?.continuousCapture)
const enabled = computed(() => Boolean(project.current?.continuousCaptureEnabled))
const busy = ref(false)
const progress = ref('')
const phase = computed(() => capture.value?.phase)
const scene = computed(() => capture.value?.plan.scenes[capture.value.sceneIndex]?.name)
let controller: AbortController | undefined

async function perform(action: (projectId: string, signal: AbortSignal) => Promise<void>) {
  const id = project.current?.id
  if (!id || busy.value) return
  busy.value = true
  progress.value = ''
  controller = new AbortController()
  let release: (() => void) | undefined
  try {
    release = project.lockActiveProject()
    await action(id, controller.signal)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '连续拍操作失败')
  } finally {
    release?.()
    busy.value = false
    controller = undefined
  }
}

async function nativeCall<T>(projectId: string, name: string, input: object = {}, signal?: AbortSignal): Promise<T> {
  const response = await fetch('/api/dramlo/execution-plan', {
    method:'POST', headers:{'X-Dramlo-UI':'1','Content-Type':'application/json'},
    body:JSON.stringify({name,input:{...input,projectId}}),signal
  })
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || '连续拍操作失败')
  return result
}

async function switchMode() {
  if (!project.current || busy.value) return
  const next = !enabled.value
  await perform(async () => {
    if (!project.current) return
    project.current.continuousCaptureEnabled = next
    project.syncCanvasFromProject()
    await project.save()
  })
}

async function importPlan(event: Event) {
  const target = event.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = ''
  if (!file) return
  if (file.size > 256 * 1024) {
    toast.warn('连续拍计划文件过大')
    return
  }
  const targetProject = project.current
  if (!targetProject) return
  const expectedUpdatedAt = targetProject.updatedAt
  const previousCaptureId = capture.value?.id
  if (capture.value && !await confirmAction('替换当前连续拍计划？旧计划的节点会移除，已生成的素材原文件会保留。')) return
  if (project.current?.id !== targetProject.id || project.current.updatedAt !== expectedUpdatedAt ||
      capture.value?.id !== previousCaptureId) {
    toast.warn('项目或连续拍计划已变化，请核对后重新导入。')
    return
  }
  await perform(async (id, signal) => {
    const plan = JSON.parse(await file.text())
    if (props.webWorkspace) {
      await nativeCall(id,'studio_capture_set_plan',{
        plan, approvalMode:approvalMode.value, expectedUpdatedAt,
        source:targetProject.script?.content
      },signal)
    } else {
      await setExternalCapturePlan(id, plan)
    }
    toast.success('连续拍计划已保存')
  })
}

async function approve() {
  const current = phase.value
  const planId = capture.value?.id
  const sceneIndex = capture.value?.sceneIndex
  const projectId = project.current?.id
  if (!planId || !projectId || (current !== 'assets' && current !== 'videos')) return
  if (!await confirmAction(current === 'assets' ? '开始生成本场缺少的素材？' : '开始按计划逐镜拍摄？')) return
  if (project.current?.id !== projectId || capture.value?.id !== planId ||
      phase.value !== current || capture.value?.sceneIndex !== sceneIndex) {
    toast.warn('连续拍计划或阶段已变化，请核对后重新确认。')
    return
  }
  await perform(async (id, signal) => {
    await approveExternalCapturePhase(id, planId, current, (event) => {
      if ('label' in event) progress.value = event.label
    }, signal)
    toast.success('连续拍阶段已保存')
  })
}

async function prepare() {
  await perform(async (id, signal) => {
    const status = await nativeCall<{capture?: {id: string; nextSceneIndex: number | null; approvalMode: string}}>(
      id,'studio_capture_state',{},signal)
    const current = status.capture
    if (!current) throw new Error('请先保存连续拍计划')
    if (current.nextSceneIndex == null) {
      toast.success('连续拍产物已齐')
      return
    }
    const result = await nativeCall<{plan?: {id: string}}>(id,'studio_capture_prepare',{
      captureId:current.id,expectedUpdatedAt:'auto',
      ...(current.approvalMode === 'scene' ? {sceneIndex:current.nextSceneIndex} : {})
    },signal)
    if (result.plan) emit('show-plan',result.plan.id)
    else toast.success('连续拍产物已齐')
  })
}

async function extractTail() {
  await perform(async (id, signal) => {
    if (props.webWorkspace) {
      const result = await nativeCall<{assetId: string}>(id,'studio_extract_tail_frame',{
        nodeId:props.selectedNodeId,expectedUpdatedAt:'auto'
      },signal)
      toast.success('尾帧已存入素材库：' + result.assetId)
    } else {
      const name = await grabLastFrame(id, props.selectedNodeId)
      toast.success('已保存尾帧：' + name)
    }
  })
}
</script>

<template>
  <section class="capture-controls">
    <h2>连续拍</h2>
    <button v-if="!webWorkspace" type="button" :disabled="busy" @click="switchMode">{{ enabled ? '关闭连续拍模式' : '开启连续拍模式' }}</button>
    <template v-if="webWorkspace || enabled">
      <p v-if="capture">{{ capture.plan.title }}{{ !webWorkspace ? ' · ' + (phase === 'assets' ? (scene || '素材准备') : phase === 'videos' ? '待拍摄' : '已完成') : '' }}</p>
      <p v-else>由外部 Agent 写入计划，也可以导入计划 JSON。</p>
      <label v-if="webWorkspace">导入时采用的确认方式
        <AppSelect v-model="approvalMode" :disabled="busy">
          <option value="batch">整批确认一次</option>
          <option value="scene">逐场确认</option>
        </AppSelect>
      </label>
      <label>导入计划 <input type="file" accept=".json" :disabled="busy" @change="importPlan" /></label>
      <button v-if="webWorkspace && capture" type="button" :disabled="busy" @click="prepare">预览执行计划</button>
      <button v-if="!webWorkspace && (phase === 'assets' || phase === 'videos')" type="button" :disabled="busy" @click="approve">
        {{ phase === 'assets' ? '确认准备本场素材' : '确认开始拍摄' }}
      </button>
      <button v-if="selectedNodeId" type="button" :disabled="busy" @click="extractTail">截取所选视频尾帧</button>
      <button v-if="busy" type="button" @click="controller?.abort()">停止本地等待</button>
      <p v-if="progress" aria-live="polite">{{ progress }}</p>
    </template>
  </section>
</template>

<style scoped>
.capture-controls { display: flex; flex-direction: column; gap: 12px; padding: 16px; overflow: auto; }
.capture-controls h2, .capture-controls p { margin: 0; }
.capture-controls button { text-align: left; padding: 7px 9px; }
</style>
