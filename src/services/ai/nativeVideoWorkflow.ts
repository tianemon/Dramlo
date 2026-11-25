import { comfyImageLoaders, fitComfyReferenceImages, guessInputMap, validateInputMap } from './comfyInputMap'
import type { AdapterConfig } from './adapters'

export type NativeVideoMode = 'reference' | 'first-frame' | 'first-last-frame' | 'previous-tail' | 'text'

type Node = { class_type: string; inputs: Record<string, unknown>; _meta?: { title?: string } }
type Graph = Record<string, Node>

function loadGraph(template: string): Graph {
  const raw = JSON.parse(template)
  const graph = raw?.prompt ?? raw
  if (!graph || typeof graph !== 'object' || Array.isArray(graph) || !Object.values(graph).length ||
    !Object.values(graph).every((node) => node && typeof (node as Node).class_type === 'string' && (node as Node).inputs)) {
    throw new Error('ComfyUI 需要 API 格式节点图')
  }
  return graph
}

export function nativeVideoBindings(adapter: AdapterConfig, mode: NativeVideoMode, imageCount: number, aspect: string) {
  const graph = loadGraph(adapter.workflowTemplate || '')
  const output = Object.values(graph).filter(({ class_type }) => /VideoCombine|SaveVideo/i.test(class_type))
  if (!output.length) throw new Error('没有找到工作流的视频输出节点')
  const fpsValues = [...new Set(output.map((node) => node.inputs.frame_rate).filter((fps) => typeof fps === 'number' && fps > 0))]
  const fps = fpsValues.length === 1 ? Number(fpsValues[0]) : undefined
  const guessed = guessInputMap(graph)
  const map = validateInputMap(graph, adapter.inputMap?.bindings.length ? adapter.inputMap : guessed)
  if (!map.bindings.some((slot) => slot.role === 'prompt')) throw new Error('工作流没有明确的正向提示词入口')
  if (!map.bindings.some((slot) => slot.role === 'length' && (slot.durationUnit === 'seconds' || (slot.durationUnit === 'frames' && fps)))) {
    const duration = guessed.bindings.find((slot) => slot.role === 'length' && slot.durationUnit === 'seconds')
    if (!duration) throw new Error('工作流没有可验证的时长入口和单位')
    map.bindings = [...map.bindings.filter((slot) => slot.role !== 'length'), duration]
  }
  const sizeMapped = ['width', 'height'].every((role) => map.bindings.some((slot) => slot.role === role))
  const fixedAspects = Object.values(graph).filter(({ class_type }) => /^ResolutionSelector$/i.test(class_type))
    .map(({ inputs }) => String(inputs.aspect_ratio || '').match(/^\s*(\d+:\d+)/)?.[1]).filter(Boolean)
  if (!sizeMapped && fixedAspects.some((value) => value !== aspect)) {
    throw new Error('工作流固定画幅为 ' + [...new Set(fixedAspects)].join('、') + '，镜头画幅为 ' + aspect)
  }

  const firstOwners = Object.values(graph).filter(({ inputs }) => Array.isArray(inputs.first_frame))
  const lastOwners = Object.values(graph).filter(({ inputs }) => Array.isArray(inputs.last_frame) || Array.isArray(inputs.end_frame))
  const referenceOwners = Object.values(graph).filter(({ inputs }) =>
    Object.entries(inputs).some(([key, value]) => /^ref_images\.ref_image_\d+$/.test(key) && Array.isArray(value)))
  if (['first-frame', 'first-last-frame', 'previous-tail'].includes(mode)) {
    const firstLoaders = new Set(firstOwners.flatMap(({ inputs }) => comfyImageLoaders(graph, inputs.first_frame)))
    if (firstLoaders.size !== 1 || referenceOwners.length) throw new Error('工作流未明确区分唯一首帧与普通参考图')
    const first = [...map.bindings, ...guessed.bindings].find((slot) => slot.role === 'image' && firstLoaders.has(slot.nodeId))
    if (!first) throw new Error('首帧连线的 LoadImage 未配置输入映射')
    const lastLoaders = new Set(lastOwners.flatMap(({ inputs }) => comfyImageLoaders(graph, inputs.last_frame ?? inputs.end_frame)))
    const last = map.bindings.find((slot) => slot.role === 'imageEnd' && lastLoaders.has(slot.nodeId))
    if (mode === 'first-last-frame' && (lastLoaders.size !== 1 || !last)) {
      throw new Error('工作流没有独立的尾帧映射')
    }
    if (mode !== 'first-last-frame' && lastLoaders.size) throw new Error('该工作流需要尾帧，请选择首尾帧模式')
    map.bindings = [...map.bindings.filter((slot) => slot.role !== 'image' && slot.role !== 'imageEnd'), first, ...(last ? [last] : [])]
  } else if (firstOwners.length || lastOwners.length) {
    throw new Error('当前 API 图包含首尾帧端口，请选择匹配的首帧模式')
  } else if (mode === 'text' && (referenceOwners.length || map.bindings.some((slot) => slot.role === 'image'))) {
    throw new Error('该工作流包含图像输入，请为当前镜头选择匹配的模式')
  }
  if (mode === 'reference' && referenceOwners.length) {
    const actualImages = guessed.bindings.filter((slot) => slot.role === 'image')
    const matched = actualImages.map((actual) => map.bindings.find((slot) =>
      slot.role === 'image' && slot.nodeId === actual.nodeId && slot.field === actual.field
    ) ?? actual)
    map.bindings = [...map.bindings.filter((slot) => slot.role !== 'image'), ...matched]
  }
  const effective = fitComfyReferenceImages(graph, map, imageCount)
  const images = effective.bindings.filter((slot) => slot.role === 'image')
  if (images.length < imageCount) throw new Error('需要 ' + imageCount + ' 张图，工作流仅可接收 ' + images.length + ' 张')
  const boundMedia = effective.bindings.filter((slot) => ['image', 'imageEnd', 'mask', 'audio'].includes(slot.role))
  if (new Set(boundMedia.map((slot) => JSON.stringify([slot.nodeId, slot.field]))).size !== boundMedia.length) {
    throw new Error('工作流有重复的媒体输入映射')
  }
  if (images.slice(0, imageCount).some((slot) => (slot.mediaMode ?? 'upload') !== 'upload') ||
    (mode === 'first-last-frame' && effective.bindings.some((slot) => slot.role === 'imageEnd' && (slot.mediaMode ?? 'upload') !== 'upload'))) {
    throw new Error('本地项目素材需要使用 ComfyUI 上传映射')
  }
  return { inputMap: map, slots: effective.bindings, sizeMapped, fps }
}
