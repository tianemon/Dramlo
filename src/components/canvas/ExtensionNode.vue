<script setup lang="ts">
import AppNumberInput from '@/components/common/AppNumberInput.vue'
import AppMedia from '@/components/common/AppMedia.vue'
import AppSelect from '@/components/common/AppSelect.vue'
import { computed, ref, watch } from 'vue'
import { Handle, Position } from '@vue-flow/core'
import { useAssetsStore, useProjectStore } from '@/stores'
import { listCanvasExtensions, mutateCanvasExtension, type CanvasExtensionParameter, type CanvasExtensionType } from '@/services/canvas/extensions'

const props = defineProps<{ id: string }>()
const project = useProjectStore()
const assets = useAssetsStore()
const definition = ref<CanvasExtensionType>()
const node = computed(() => project.current?.canvas?.nodes.find(item => item.id === props.id))
const busy = ref(false)
const error = ref('')
const preview = ref<string>()
const shotId = ref('')
const mode = ref('reference')
const prepared = ref(false)
const toolArguments = ref('{}')
const formRevision = ref(0)
const mcp = computed(() => definition.value?.executor === 'studio.mcp')
const comfy = computed(() => definition.value?.executor === 'studio.comfy-video')

watch([() => node.value?.data.extensionType, () => node.value?.data.extensionVersion, () => project.current?.id], async ([id, version], _, onCleanup) => {
  let active = true
  onCleanup(() => { active = false })
  definition.value = undefined
  try {
    const items = await listCanvasExtensions()
    if (active) definition.value = items.find(item => item.id === id && item.version === version)
  } catch (failure) {
    if (active) error.value = failure instanceof Error ? failure.message : String(failure)
  }
}, { immediate: true })

function paramValue(param: CanvasExtensionParameter) {
  return node.value?.data.extensionParams?.[param.key] ?? param.default
}

async function updateParam(param: CanvasExtensionParameter, value: string | number | boolean | null) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await mutateCanvasExtension('studio_update_extension_node', { nodeId: props.id, params: { [param.key]: value } })
    prepared.value = false
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : String(failure)
    formRevision.value++
  } finally {
    busy.value = false
  }
}

function changeParam(param: CanvasExtensionParameter, event: Event) {
  const control = event.target as HTMLInputElement
  if (param.type === 'boolean') {
    void updateParam(param, control.checked)
  } else if (param.type === 'number' || param.type === 'integer') {
    if (!control.checkValidity()) {
      error.value = '请输入参数允许范围内的数值。'
      return
    }
    void updateParam(param, control.value === '' ? null : control.valueAsNumber)
  } else {
    void updateParam(param, control.value === '' ? null : control.value)
  }
}

watch(() => node.value?.data.outputAssetId, async (id, _, onCleanup) => {
  let active = true
  onCleanup(() => { active = false })
  const url = id ? await assets.getUrl(id).catch(() => undefined) : undefined
  if (active) preview.value = url
}, { immediate: true })

async function run() {
  busy.value = true
  error.value = ''
  try {
    if (comfy.value) {
      await mutateCanvasExtension('studio_prepare_comfy_node', {
        nodeId: props.id, requestId: crypto.randomUUID(), shot: { shotId: shotId.value, mode: mode.value }
      })
      prepared.value = true
    } else await mutateCanvasExtension('studio_run_extension_node', {
      nodeId: props.id, ...(mcp.value ? { arguments: JSON.parse(toolArguments.value) } : {})
    })
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <section class="extension-card">
    <strong>{{ definition?.name || node?.data.label || node?.data.extensionType }}</strong>
    <template v-if="definition">
      <p>{{ definition.description }}</p>
      <Handle v-for="(port, index) in definition.inputs" :id="port.id" :key="'input-' + port.id" type="target" :position="Position.Left"
        :title="port.id + ' · ' + port.kind" :style="{ top: (100 * (index + 1) / (definition.inputs.length + 1)) + '%' }" />
      <Handle v-for="port in definition.outputs" :id="port.id" :key="'output-' + port.id" type="source" :position="Position.Right" :title="port.id + ' · ' + port.kind" />
      <div v-for="param in definition.params" :key="param.key + ':' + formRevision" class="param-row nodrag">
        <label>{{ param.label }}
          <AppSelect v-if="param.enum?.length" :value="paramValue(param) === undefined ? '' : param.enum.indexOf(paramValue(param)!)"
            :disabled="busy" @change="updateParam(param, $event === '' ? null : param.enum?.[Number($event)] ?? null)">
            <option value="">使用默认值</option>
            <option v-for="(value, index) in param.enum" :key="index" :value="index">{{ value }}</option>
          </AppSelect>
          <input v-else-if="param.type === 'boolean'" type="checkbox" :checked="paramValue(param) === true"
            :disabled="busy" @change="changeParam(param, $event)" />
          <AppNumberInput v-else-if="param.type === 'number' || param.type === 'integer'" :min="param.min" :max="param.max"
            :step="param.type === 'integer' ? 1 : 'any'" :value="paramValue(param)" :disabled="busy" @change="changeParam(param, $event)" />
          <input v-else type="text" :value="paramValue(param)" :disabled="busy" @change="changeParam(param, $event)" />
        </label>
        <button v-if="node?.data.extensionParams?.[param.key] !== undefined" type="button" :disabled="busy" @click="updateParam(param, null)">恢复默认</button>
      </div>
      <template v-if="comfy">
        <label>分镜
          <AppSelect v-model="shotId" class="nodrag">
            <option value="" disabled>选择分镜</option>
            <option v-for="shot in project.current?.shots" :key="shot.id" :value="shot.id">{{ shot.title || shot.id }}</option>
          </AppSelect>
        </label>
        <label>输入
          <AppSelect v-model="mode" class="nodrag">
            <option value="reference">参考图</option>
            <option value="first-frame">首帧</option>
            <option value="first-last-frame">首尾帧</option>
            <option value="previous-tail">上一镜尾帧</option>
            <option value="text">仅文本</option>
          </AppSelect>
        </label>
        <p v-if="prepared" role="status">预检完成，在画布上方的制作批次核对并确认。</p>
      </template>
      <label v-if="mcp">工具参数 JSON <textarea v-model="toolArguments" rows="3" class="nodrag" /></label>
      <button class="nodrag" type="button" :disabled="busy || (comfy && !shotId)" @click="run">{{ busy ? '处理中' : comfy ? '预检' : '运行' }}</button>
    </template>
    <p v-else>当前节点扩展不可用</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <pre v-if="node?.data.extensionResult">{{ node.data.extensionResult }}</pre>
    <AppMedia v-if="preview && node?.data.extensionExecutor === 'studio.comfy-video'" :src="preview" class="nodrag" />
    <img v-else-if="preview" :src="preview" alt="视频尾帧" />
  </section>
</template>

<style scoped>
.extension-card { width: 240px; padding: 14px; border: 1px solid var(--color-border); border-radius: 10px; background: var(--color-bg-2); }
p { margin: 8px 0; font-size: 12px; }
pre { max-height: 160px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
label { display: grid; gap: 4px; margin: 8px 0; }
.param-row { display: flex; align-items: end; gap: 6px; }
.param-row label { flex: 1; min-width: 0; }
.param-row input:not([type=checkbox]), .param-row .app-number, .param-row .app-select { min-width: 0; width: 100%; }
.param-row button { margin-bottom: 8px; flex-shrink: 0; }
img, .app-media-video { display: block; width: 100%; margin-top: 10px; border-radius: 5px; }
</style>
