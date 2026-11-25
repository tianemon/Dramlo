<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import ExtensionNode from "@/components/canvas/ExtensionNode.vue"
import { listCanvasExtensions, mutateCanvasExtension, type CanvasExtensionType } from "@/services/canvas/extensions"
import { computed, defineComponent, h, markRaw, nextTick, onBeforeMount, onMounted, onUnmounted, ref, watch, type Component } from 'vue'
import { useRouter } from 'vue-router'
import { SelectionMode, VueFlow, useVueFlow, type Connection, type Edge, type Node, type NodeMouseEvent } from '@vue-flow/core'
import { Controls } from '@vue-flow/controls'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import '@vue-flow/controls/dist/style.css'
import { useProjectStore, useAssetsStore, useToastStore } from '@/stores'
import { arrangeCanvasNodes, rebuildCanvasGraph, AUDIO_NODE_H, NODE_W, pipelineNodeHeight, nextFreeformPosition, rescueStrandedFreeformNodes, canvasNodeSize } from '@/services/canvas/layout'
import { setCanvasReveal, setCanvasInspect, setCanvasSelect, setCanvasLinkMenu } from '@/services/canvas/canvasBridge'
import { rebindShotScenes } from '@/services/canvas/runner'
import {
  appendCanvasReferenceSource,
  canvasNodeOutputAssetId,
  canvasNodeProducesImage,
  carryTextFromNode,
  nextCanvasLinks,
  prevCanvasLinks,
  type LinkCanvasKind
} from '@/services/canvas/references'
import { isDesktop, desktop } from '@/services/desktop'
import PipelineNode from '@/components/canvas/PipelineNode.vue'
import ExecutionPlanPanel from '@/components/canvas/ExecutionPlanPanel.vue'
import CaptureTextNode from '@/components/canvas/CaptureTextNode.vue'
import ResizableNode from '@/components/canvas/ResizableNode.vue'
import { normalizeNodeScale } from '@/utils/nodeResize'
import AudioNode from '@/components/canvas/AudioNode.vue'
import ScriptDocNode from '@/components/canvas/ScriptDocNode.vue'
import GroupFrame from '@/components/canvas/GroupFrame.vue'
import FreeformGenNode from '@/components/canvas/FreeformGenNode.vue'
import AudioGenNode from '@/components/canvas/AudioGenNode.vue'
import ComposeNode from '@/components/canvas/ComposeNode.vue'
import CtxMenuIcon from '@/components/canvas/CtxMenuIcon.vue'
import ContinuousCaptureControls from '@/components/canvas/ContinuousCaptureControls.vue'
import StudioVideoStatus from '@/components/canvas/StudioVideoStatus.vue'
import StepNav from '@/components/layout/StepNav.vue'
import HomeIcon from '@/components/common/HomeIcon.vue'
import { removeWebShot } from '@/services/localWebStorage'
import CanvasGuide from '@/components/canvas/CanvasGuide.vue'
import NodeInspector from '@/components/canvas/NodeInspector.vue'
import CanvasToolbarIcon from '@/components/canvas/CanvasToolbarIcon.vue'
import { type CanvasNode, type Shot } from '@/types'

const project = useProjectStore()
const assets = useAssetsStore()
const toast = useToastStore()
const router = useRouter()
const agentOpen = ref(false)
const executionPanel = ref<InstanceType<typeof ExecutionPlanPanel> | null>(null)
const webWorkspace = isDesktop()
const initialCapturePanel = computed(() => project.current?.continuousCaptureEnabled && !window.desktop)
const GUIDE_KEY = 'aicut:guide:canvas'
const guideOpen = ref(!webWorkspace && localStorage.getItem(GUIDE_KEY) !== 'done')
function dismissGuide() {
  guideOpen.value = false
  try {
    localStorage.setItem(GUIDE_KEY, 'done')
  } catch {
    /* 本地存储不可用时，本次会话内也不再弹 */
  }
}

const agentWidth = ref(440)
const resizingAgent = ref(false)
const rightOpen = ref(false)
// 合并模式：不再区分自由画布与创作模式，只有这一种画布
// （保留 img2video 只是老项目的历史字段，不再用于功能分叉）

const AGENT_WIDTH_KEY = 'aicut:agent-chat-width'
const AGENT_MIN_WIDTH = 300
const AGENT_MAX_WIDTH = 760
let resizeStartX = 0
let resizeStartWidth = 440
let previousBodyCursor = ''
let previousBodyUserSelect = ''

function maxAgentWidth() {
  const rightDock = rightOpen.value ? 300 : 0
  return Math.max(
    AGENT_MIN_WIDTH,
    Math.min(AGENT_MAX_WIDTH, window.innerWidth - rightDock - 360)
  )
}

function clampAgentWidth(value: number) {
  return Math.round(Math.max(AGENT_MIN_WIDTH, Math.min(maxAgentWidth(), value)))
}

function persistAgentWidth() {
  try {
    localStorage.setItem(AGENT_WIDTH_KEY, String(agentWidth.value))
  } catch {
    /* localStorage unavailable */
  }
}

function loadAgentWidth() {
  try {
    const saved = Number(localStorage.getItem(AGENT_WIDTH_KEY))
    if (Number.isFinite(saved) && saved > 0) agentWidth.value = clampAgentWidth(saved)
  } catch {
    agentWidth.value = clampAgentWidth(agentWidth.value)
  }
}

function onAgentResizeMove(event: PointerEvent) {
  if (!resizingAgent.value) return
  agentWidth.value = clampAgentWidth(resizeStartWidth + event.clientX - resizeStartX)
}

function stopAgentResize() {
  if (!resizingAgent.value) return
  resizingAgent.value = false
  window.removeEventListener('pointermove', onAgentResizeMove)
  window.removeEventListener('pointerup', stopAgentResize)
  window.removeEventListener('pointercancel', stopAgentResize)
  document.body.style.cursor = previousBodyCursor
  document.body.style.userSelect = previousBodyUserSelect
  persistAgentWidth()
}

function startAgentResize(event: PointerEvent) {
  if (event.button !== 0) return
  event.preventDefault()
  resizeStartX = event.clientX
  resizeStartWidth = agentWidth.value
  resizingAgent.value = true
  previousBodyCursor = document.body.style.cursor
  previousBodyUserSelect = document.body.style.userSelect
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('pointermove', onAgentResizeMove)
  window.addEventListener('pointerup', stopAgentResize)
  window.addEventListener('pointercancel', stopAgentResize)
}

function nudgeAgentWidth(delta: number) {
  agentWidth.value = clampAgentWidth(agentWidth.value + delta)
  persistAgentWidth()
}

function clampAgentToViewport() {
  const next = clampAgentWidth(agentWidth.value)
  if (next !== agentWidth.value) agentWidth.value = next
}

watch(rightOpen, clampAgentToViewport)

const extensionTypes = ref<CanvasExtensionType[]>([])
watch(() => project.current?.id, async (_, __, onCleanup) => {
  if (!webWorkspace) return
  let active = true
  onCleanup(() => { active = false })
  extensionTypes.value = []
  const items = await listCanvasExtensions().catch(() => [])
  if (active) extensionTypes.value = items
}, { immediate: true })

async function addExtension(type: CanvasExtensionType) {
  const menu = ctxMenu.value
  if (!menu) return
  closeCtxMenu()
  try {
    await mutateCanvasExtension('studio_add_extension_node', {
      typeId: type.id, version: type.version, nodeId: crypto.randomUUID(), ...menu.flow
    })
  } catch (failure) {
    toast.warn(failure instanceof Error ? failure.message : String(failure))
  }
}

const rawNodeTypes = {
  extension: ExtensionNode,
  captureText: CaptureTextNode,
  script: ScriptDocNode,
  subject: PipelineNode,
  prop: PipelineNode,
  subjectProps: PipelineNode,
  scene: PipelineNode,
  sceneProps: PipelineNode,
  shotBoard: PipelineNode,
  shotImage: PipelineNode,
  shotFrames: PipelineNode,
  shotVideo: PipelineNode,
  audio: AudioNode,
  export: PipelineNode,
  final: PipelineNode,
  groupFrame: GroupFrame,
  imageGen: FreeformGenNode,
  videoGen: FreeformGenNode,
  audioGen: AudioGenNode,
  compose: ComposeNode
}
const nodeTypes = Object.fromEntries(Object.entries(rawNodeTypes).map(([type, renderer]) => [type, type === 'groupFrame' ? markRaw(renderer) : markRaw(defineComponent({
  inheritAttrs: false,
  setup(_, { attrs }) { return () => h(ResizableNode, { ...attrs, id: String(attrs.id), data: attrs.data as any, renderer: markRaw(renderer as Component) }) }
}))]))

const { onConnect, onNodeDragStop, onMoveEnd, fitView, screenToFlowCoordinate, getSelectedNodes, removeSelectedNodes } = useVueFlow()

const ctxMenu = ref<{
  x: number
  y: number
  flow: { x: number; y: number }
  nodeId?: string
  onNode?: boolean
  linkOnly?: boolean
  linkSide?: 'prev' | 'next'
  links?: LinkCanvasKind[]
} | null>(null)

function imageRowGrowth(p: NonNullable<typeof project.current>, n: CanvasNode) {
  if ((!canvasNodeProducesImage(n) && n.type !== 'shotVideo') || n.type === 'shotFrames') return 0
  const id = canvasNodeOutputAssetId(p, n)
  const asset = id ? assets.getMeta(id) : undefined
  const defaultAspect = n.type === 'shotImage' || n.type === 'shotBoard' || n.type === 'shotVideo' ? p.settings.aspectRatio : '16:9'
  const imageAspect = asset?.kind === 'image' && asset.width > 0 && asset.height > 0 ? `${asset.width}:${asset.height}` : defaultAspect
  return Math.max(0, pipelineNodeHeight(imageAspect) * normalizeNodeScale(n.data.canvasScale) - pipelineNodeHeight(defaultAspect))
}

// Expand rendered rows for taller images without rewriting saved canvas coordinates.
const imageRowLayout = computed(() => {
  const offsets = new Map<string, number>()
  const growth = new Map<string, number>()
  const p = project.current
  const nodes = p?.canvas?.nodes
  if (!p || !nodes) return { offsets, growth }
  const byGroup = new Map<string, CanvasNode[]>()
  for (const n of nodes) {
    if (!n.parentGroupId) continue
    const children = byGroup.get(n.parentGroupId) ?? []
    children.push(n)
    byGroup.set(n.parentGroupId, children)
  }
  for (const [group, children] of byGroup) {
    let extra = 0
    for (const n of children.sort((a, b) => a.position.y - b.position.y)) {
      offsets.set(n.id, extra)
      extra += imageRowGrowth(p, n)
    }
    growth.set(group, extra)
  }
  return { offsets, growth }
})

const flowNodes = computed<Node[]>(() => {
  const graph = project.current?.canvas
  if (!graph) return []
  return graph.nodes.map((n) => {
    if (n.type === 'groupFrame') {
      return {
        id: n.id,
        type: 'groupFrame',
        position: n.position,
        data: { ...n.data, nodeType: n.type },
        style: {
          width: `${n.data.width ?? 280}px`,
          height: `${(n.data.height ?? 240) + (imageRowLayout.value.growth.get(n.id) ?? 0)}px`
        },
        draggable: true,
        selectable: false,
        zIndex: 0
      }
    }
    if (n.type === 'script') {
      return {
        id: n.id,
        type: 'script',
        position: n.position,
        data: { ...n.data, nodeType: n.type },
        style: {
          width: `${(n.data.width ?? 460) * normalizeNodeScale(n.data.canvasScale)}px`,
          height: `${(n.data.height ?? 720) * normalizeNodeScale(n.data.canvasScale)}px`
        },
        draggable: true,
        zIndex: 2
      }
    }
    const mapped: Node = {
      id: n.id,
      type: n.type,
      position: { x: n.position.x, y: n.position.y + (imageRowLayout.value.offsets.get(n.id) ?? 0) },
      data: { ...n.data, nodeType: n.type },
      draggable: true,
      zIndex: n.type === 'imageGen' || n.type === 'videoGen' || n.type === 'audioGen' || n.type === 'compose' ? 12 : 1
    }
    if (n.parentGroupId) {
      mapped.parentNode = n.parentGroupId
      mapped.expandParent = false
    }
    return mapped
  })
})

const flowEdges = computed<Edge[]>(() => {
  const graph = project.current?.canvas
  if (!graph) return []
  return graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    sourceHandle: e.sourceHandle,
    targetHandle: e.targetHandle,
    deletable: e.deletable,
    animated: false,
    interactionWidth: 28,
    class: 'pipe-edge',
    style: { stroke: 'var(--nb-edge)', strokeWidth: 1.4 }
  }))
})

const selected = computed<CanvasNode | null>(() => {
  const id = selectedId.value
  if (!id || !project.current?.canvas) return null
  return project.current.canvas.nodes.find((n) => n.id === id) ?? null
})
const selectedId = ref<string | null>(null)
const selectionIds = ref<string[]>([])
const selectionMenu = ref<{ x: number; y: number; actions: Array<{ id: string; label: string }> } | null>(null)
const REF_LIMIT = 9

function flowSelection(): CanvasNode[] {
  const nodes = project.current?.canvas?.nodes || []
  return getSelectedNodes.value
    .map((item) => nodes.find((node) => node.id === item.id))
    .filter((node): node is CanvasNode => Boolean(node))
}

function isPickText(node: CanvasNode) {
  return node.type === 'script' || node.type === 'captureText'
}

function isPickVideo(node: CanvasNode) {
  return node.type === 'videoGen' || node.type === 'shotVideo' || node.type === 'compose'
}

function byReadingOrder(nodes: CanvasNode[]) {
  return [...nodes].sort((a, b) => a.position.x - b.position.x || a.position.y - b.position.y)
}

function placeSelectionMenu(event: Event | undefined, actions: Array<{ id: string; label: string }>) {
  const point = event && 'clientX' in event ? (event as MouseEvent) : undefined
  selectionMenu.value = {
    x: point ? Math.max(8, Math.min(point.clientX, window.innerWidth - 216)) : 24,
    y: point ? Math.max(8, Math.min(point.clientY, window.innerHeight - 240)) : 24,
    actions
  }
}

function onBoxSelect(event?: MouseEvent) {
  const picked = flowSelection()
  if (picked.length < 2) {
    selectionIds.value = []
    selectionMenu.value = null
    return
  }
  const texts = picked.filter(isPickText)
  if (texts.length > 1) toast.info('文字一次只带第一段，删除会带上全部选中的节点')
  selectionIds.value = picked.map((node) => node.id)
  const nodes = texts.length > 1
    ? picked.filter((node) => !texts.slice(1).some((text) => text.id === node.id))
    : picked
  const videos = byReadingOrder(nodes.filter(isPickVideo))
  const images = byReadingOrder(nodes.filter((node) => canvasNodeProducesImage(node)))
  const text = texts[0]
  const actions: Array<{ id: string; label: string }> = []
  if (videos.length >= 2 && !images.length) {
    actions.push({ id: 'compose', label: '合成视频' }, { id: 'download', label: '批量下载' })
  } else if (images.length && videos.length === 1) {
    actions.push({ id: 'attach', label: '加入参考图' })
  } else if (images.length && !videos.length) {
    actions.push({ id: 'video', label: '添加视频节点' }, { id: 'image', label: '生图节点' })
  } else if (videos.length === 1 && text && !images.length) {
    actions.push({ id: 'attach', label: '加入文字' })
  } else if (videos.length >= 2) {
    actions.push({ id: 'compose', label: '合成视频' }, { id: 'download', label: '批量下载' })
  }
  placeSelectionMenu(event, actions)
}

function selectionNodes() {
  const all = project.current?.canvas?.nodes || []
  const nodes = selectionIds.value
    .map((id) => all.find((node) => node.id === id))
    .filter((node): node is CanvasNode => Boolean(node))
  return {
    videos: byReadingOrder(nodes.filter(isPickVideo)),
    images: byReadingOrder(nodes.filter((node) => canvasNodeProducesImage(node))).slice(0, REF_LIMIT),
    text: nodes.find(isPickText)
  }
}

function connectNodes(pairs: Array<[string, string]>) {
  const canvas = project.current?.canvas
  if (!canvas || !pairs.length) return
  const edges = [...canvas.edges]
  for (const [source, target] of pairs) {
    const id = `e-${source}-${target}-in`
    if (!edges.some((edge) => edge.id === id || (edge.source === source && edge.target === target))) {
      edges.push({ id, source, target, sourceHandle: 'out', targetHandle: 'in' })
    }
  }
  project.setCanvas({ ...canvas, edges })
}

function composeSelection() {
  const { videos } = selectionNodes()
  if (videos.length < 2 || !project.current) return
  const last = videos[videos.length - 1]
  const id = `compose_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
  project.addCanvasNode({
    id,
    type: 'compose',
    position: { x: last.position.x + 280, y: last.position.y },
    data: { label: '合成' },
    status: 'idle'
  })
  connectNodes(videos.map((node) => [node.id, id]))
  selectedId.value = id
  selectionMenu.value = null
  selectionIds.value = []
  toast.success(`已把 ${videos.length} 段视频接到合成节点`)
}

async function downloadSelection() {
  const { videos } = selectionNodes()
  let saved = 0
  for (const node of videos) {
    const assetId = project.current ? canvasNodeOutputAssetId(project.current, node) : undefined
    const blob = assetId ? await assets.getBlob(assetId) : undefined
    if (!blob) continue
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = `${(node.data.label || '视频').replace(/[\\/:*?"<>|]/g, ' ')}.mp4`
    link.click()
    URL.revokeObjectURL(url)
    saved += 1
    await new Promise((resolve) => setTimeout(resolve, 180))
  }
  toast[saved ? 'success' : 'warn'](saved ? `开始下载 ${saved} 个视频` : '这些视频还没有成片')
}

function addFromSelection(type: 'imageGen' | 'videoGen') {
  if (!project.current) return
  const { images, text } = selectionNodes()
  if (!images.length) return
  const count = (project.current.canvas?.nodes ?? []).filter((node) => node.type === type).length + 1
  const id = `${type}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
  const anchor = images[images.length - 1]
  project.addCanvasNode({
    id,
    type,
    position: nextFreeformPosition(project.current.canvas?.nodes ?? [], { source: anchor, place: 'after' }),
    status: 'idle',
    data: {
      label: type === 'videoGen' ? `视频 ${count}` : `图片 ${count}`,
      prompt: text ? carryTextFromNode(project.current, text) : '',
      referenceAssetIds: [],
      referenceSourceNodeIds: images.map((node) => node.id),
      aspectRatio: project.current.settings.aspectRatio,
      ...(type === 'videoGen'
        ? { duration: 8, videoModel: 'h3' as const, videoResolution: project.current.settings.defaultVideoResolution || '768p' }
        : {})
    }
  })
  connectNodes([
    ...images.map((node) => [node.id, id] as [string, string]),
    ...(text ? [[text.id, id] as [string, string]] : [])
  ])
  selectedId.value = id
  selectionMenu.value = null
  selectionIds.value = []
  toast.success(type === 'videoGen' ? `已添加视频节点，带上 ${images.length} 张参考图` : `已添加生图节点，带上 ${images.length} 张参考图`)
}

function attachSelection() {
  if (!project.current) return
  const { videos, images, text } = selectionNodes()
  const video = videos[0]
  if (!video || (!images.length && !text)) return
  const before = new Set(video.data.referenceSourceNodeIds || [])
  let added = 0
  for (const image of images) {
    if (appendCanvasReferenceSource(project.current, video.id, image.id) && !before.has(image.id)) added += 1
  }
  const prompt = text ? carryTextFromNode(project.current, text) : ''
  if (prompt) {
    const existing = video.data.prompt || ''
    project.setCanvasNodePrompt(video.id, existing.includes(prompt) ? existing : `${existing}\n${prompt}`.trim())
  }
  connectNodes([
    ...images.map((node) => [node.id, video.id] as [string, string]),
    ...(text ? [[text.id, video.id] as [string, string]] : [])
  ])
  project.patchCanvasNode(video.id, { referenceSourceNodeIds: [...(video.data.referenceSourceNodeIds || [])] })
  selectedId.value = video.id
  selectionMenu.value = null
  selectionIds.value = []
  toast.success(
    added && prompt
      ? `已把 ${added} 张图和文字加入这个视频`
      : added
        ? `已把 ${added} 张图加入参考`
        : prompt
          ? '已把文字加入这个视频'
          : '这些图已经在参考里了'
  )
}

function runSelectionAction(id: string) {
  if (id === 'compose') composeSelection()
  else if (id === 'download') void downloadSelection()
  else if (id === 'video') addFromSelection('videoGen')
  else if (id === 'image') addFromSelection('imageGen')
  else if (id === 'attach') attachSelection()
}

onBeforeMount(() => {
  if (!project.current) return
  const hasPipeline =
    (project.current.subjects?.length || 0) > 0 ||
    (project.current.scenes?.length || 0) > 0 ||
    project.current.shots.length > 0 ||
    Boolean(project.current.canvas?.nodes.some((n) => n.type === 'groupFrame'))
  // 空画布由用户从入口或右键添加节点。
  const hasGroupFrame = project.current.canvas?.nodes.some((n) => n.type === 'groupFrame')
  if (hasPipeline || hasGroupFrame) {
    project.setCanvas(rebuildCanvasGraph(project.current, Boolean(hasGroupFrame)))
  }
  rebindShotScenes()
  pullStrandedFreeformIntoView()
})

onMounted(async () => {
  if (!project.current) {
    toast.warn('请先打开项目')
    router.push('/')
    return
  }

  agentOpen.value = Boolean(initialCapturePanel.value)
  loadAgentWidth()
  await assets.refresh()
  document.querySelectorAll('body > input[type="file"]').forEach((el) => el.remove())
  setCanvasReveal((nodeId) => {
    setTimeout(() => {
      if (nodeId) fitView({ nodes: [nodeId], padding: 0.28, duration: 450 })
      else fitView({ padding: 0.2, duration: 400 })
    }, 120)
  })
  setCanvasInspect((nodeId) => {
    selectedId.value = nodeId
    rightOpen.value = true
  })
  setCanvasSelect((nodeId) => {
    selectedId.value = nodeId
  })
  setCanvasLinkMenu((nodeId, x, y, side) => openNodeLinkMenu(nodeId, x, y, side))
  setTimeout(() => fitView({ padding: 0.2 }), 80)
  window.addEventListener('keydown', onDupKey)
  window.addEventListener('keydown', keepSpaceForSelection)
  window.addEventListener('resize', clampAgentToViewport)
})

onUnmounted(() => {
  stopAgentResize()
  window.removeEventListener('keydown', onDupKey)
  window.removeEventListener('keydown', keepSpaceForSelection)
  window.removeEventListener('resize', clampAgentToViewport)
  setCanvasReveal(null)
  setCanvasInspect(null)
  setCanvasSelect(null)
  setCanvasLinkMenu(null)
})

function canvasBinding(sourceId: string | null, targetId: string | null) {
  const nodes = project.current?.canvas?.nodes ?? []
  const source = nodes.find(node => node.id === sourceId)
  const target = nodes.find(node => node.id === targetId)
  if (!source || !target?.data.shotId || target.data.captureId ||
      !['shotBoard', 'shotImage', 'shotFrames', 'shotVideo'].includes(target.type)) return
  const kind = source.type
  if (kind !== 'subject' && kind !== 'scene' && kind !== 'prop') return
  const id = kind === 'subject' ? source.data.subjectId : kind === 'scene' ? source.data.sceneId : source.data.propId
  if (id) return { kind, id, shotId: target.data.shotId }
}

async function changeCanvasBinding(sourceId: string, targetId: string, connected: boolean) {
  const binding = canvasBinding(sourceId, targetId)
  if (!binding) return
  try {
    await project.changeShotBinding(binding.shotId, binding.kind, binding.id, connected)
    toast.success(connected ? '已保存镜头绑定' : '已断开镜头绑定')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '未能保存镜头绑定')
  }
}

async function copySelectedForAgent() {
  const current = project.current
  const node = selected.value
  if (!current || !node) return
  try {
    if (project.dirty) await project.save()
    if (project.current?.id !== current.id || project.dirty) throw new Error('项目状态已变化，请重试')
    await navigator.clipboard.writeText(JSON.stringify({
      projectId: current.id, nodeId: node.id, shotId: node.data.shotId, updatedAt: project.current.updatedAt
    }))
    toast.success('已复制选中对象，可粘贴给 Agent')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '未能复制选中对象')
  }
}

onConnect((c: Connection) => {
  if (!project.current?.canvas) return
  if (c.targetHandle === 'binding') {
    if (c.source && c.target) void changeCanvasBinding(c.source, c.target, true)
    return
  }
  const source = project.current.canvas.nodes.find((node) => node.id === c.source)
  const target = project.current.canvas.nodes.find((node) => node.id === c.target)
  if (source && target && (source.type === 'extension' || target.type === 'extension' || c.targetHandle === 'first-frame')) {
    void mutateCanvasExtension('studio_connect_extension_node', { sourceId: source.id, targetId: target.id,
      sourceHandle: c.sourceHandle || undefined, targetHandle: c.targetHandle || undefined })
      .catch(failure => toast.warn(failure instanceof Error ? failure.message : String(failure)))
    return
  }
  if (
    source &&
    target &&
    canvasNodeProducesImage(source) &&
    (target.type === 'imageGen' || target.type === 'videoGen') &&
    !appendCanvasReferenceSource(project.current, target.id, source.id)
  ) {
    toast.warn('参考图数量已达到当前模型上限')
    return
  }
  const id = `e-${c.source}-${c.target}-${c.targetHandle || 'in'}`
  const edges = [
    ...project.current.canvas.edges.filter((e) => e.id !== id),
    {
      id,
      source: c.source,
      target: c.target,
      sourceHandle: c.sourceHandle || undefined,
      targetHandle: c.targetHandle || undefined
    }
  ]
  project.setCanvas({ ...project.current.canvas, edges })
})

onNodeDragStop(({ node }) => {
  if (!project.current?.canvas || !node) return
  if (node.type === 'groupFrame') {
    // 拖分组框：成员按本次位移同步移动
    const prev = project.current.canvas.nodes.find((n) => n.id === node.id)
    const dx = node.position.x - (prev?.position.x ?? node.position.x)
    const dy = node.position.y - (prev?.position.y ?? node.position.y)
    const nodes = project.current.canvas.nodes.map((n) => {
      if (n.id === node.id) return { ...n, position: { ...node.position } }
      if (n.parentGroupId === node.id) {
        return { ...n, position: { x: n.position.x + dx, y: n.position.y + dy } }
      }
      return n
    })
    project.setCanvas({ ...project.current.canvas, nodes })
    return
  }
  // 只保存位置：归属由节点类型决定，拖到哪都不会换组
  const stored = project.current.canvas.nodes.find((n) => n.id === node.id)
  const position = { ...node.position }
  if (stored?.parentGroupId) position.y -= imageRowLayout.value.offsets.get(node.id) ?? 0
  const nodes = project.current.canvas.nodes.map((n) =>
    n.id === node.id ? { ...n, position } : n
  )
  project.setCanvas({ ...project.current.canvas, nodes })
})

onMoveEnd((ev) => {
  const vp = ev?.flowTransform
  if (!project.current?.canvas || !vp) return
  project.setCanvas({
    ...project.current.canvas,
    viewport: { x: vp.x, y: vp.y, zoom: vp.zoom }
  })
})

function onNodeClick(ev: { node: Node }) {
  if (ev.node.type === 'groupFrame') return
  selectedId.value = ev.node.id
  if (ev.node.type === 'export' || ev.node.type === 'final') rightOpen.value = true
}

const arranging = ref(false)

function handleGlobalPointerDown() {
  if (ctxMenu.value) closeCtxMenu()
}
async function relayout() {
  const current = project.current
  if (!current?.canvas?.nodes.length || arranging.value) return
  arranging.value = true
  try {
    project.setCanvas(
      arrangeCanvasNodes(current, (node) => canvasNodeSize(current, node, (id) => assets.getMeta(id)))
    )
    await project.save()
    if (project.current?.id !== current.id) return
    await nextTick()
    await fitView({ padding: 0.15, duration: 240 })
    toast.success('节点已整理')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '节点布局保存失败')
  } finally {
    arranging.value = false
  }
}

function eventFromCtx(
  ev: MouseEvent | TouchEvent | { event: MouseEvent | TouchEvent }
): MouseEvent | TouchEvent | null {
  if (ev instanceof MouseEvent || ev instanceof TouchEvent) return ev
  if (ev && typeof ev === 'object' && ev.event) return ev.event
  return null
}

function pointerClient(ev: MouseEvent | TouchEvent): { x: number; y: number } | null {
  if (ev instanceof MouseEvent) return { x: ev.clientX, y: ev.clientY }
  const t = ev.changedTouches[0] || ev.touches[0]
  if (!t) return null
  return { x: t.clientX, y: t.clientY }
}

function canDuplicate(id: string | null | undefined): boolean {
  if (!id) return false
  const n = project.current?.canvas?.nodes.find((x) => x.id === id)
  return n?.type === 'imageGen' || n?.type === 'videoGen' || n?.type === 'audioGen'
}

function canDeleteNode(id: string | null | undefined): boolean {
  if (!id) return false
  const n = project.current?.canvas?.nodes.find((x) => x.id === id)
  if (!n) return false
  return n.type !== 'groupFrame' && n.type !== 'export' && n.type !== 'final'
}

function duplicateNode(id: string | null | undefined) {
  if (!id) {
    toast.warn('请先选中要复制的图片、视频或音频节点')
    return
  }
  const nid = project.duplicateCanvasNode(id)
  closeCtxMenu()
  if (!nid) {
    toast.warn('只能复制图片、视频或音频节点')
    return
  }
  selectedId.value = nid
  toast.success('已复制节点')
  nextTick(() => {
    fitView({ nodes: [nid], padding: 0.35, duration: 280 })
  })
}

async function deleteSelection() {
  const ids = selectionIds.value.filter((id) => canDeleteNode(id))
  if (!ids.length || !project.current) return
  if (!await confirmAction(`删除选中的 ${ids.length} 个节点？其中镜头节点会移除整镜及其剪辑关联，原素材保留。`)) return
  let removed = 0
  for (const id of ids) if (await deleteCtxNode(id, { skipConfirm: true, quiet: true })) removed++
  if (selectedId.value && ids.includes(selectedId.value)) selectedId.value = null
  selectionMenu.value = null
  selectionIds.value = []
  removeSelectedNodes(getSelectedNodes.value.filter((node) => ids.includes(node.id)))
  if (removed) toast.success(`已处理 ${removed} 个节点`)
}

async function deleteCtxNode(id: string | null | undefined, options?: { skipConfirm?: boolean; quiet?: boolean }) {
  if (!id || !project.current) return
  const n = project.current.canvas?.nodes.find((x) => x.id === id)
  if (!n || !canDeleteNode(id)) {
    closeCtxMenu()
    return
  }
  const removesShot = ['shotImage','shotBoard','shotVideo','shotFrames','audio'].includes(n.type) && n.data.shotId
  if (!options?.skipConfirm && !await confirmAction(removesShot ? '移除整镜及其画布和剪辑关联？原素材保留。' : '删除这个节点？')) {
    closeCtxMenu()
    return
  }
  const p = project.current
  if (removesShot && window.desktop?.isLocalWeb) {
    try {
      if (project.dirty) await project.save()
      if (project.current?.id !== p.id || project.dirty) throw new Error('项目已切换或尚未保存')
      const result = await removeWebShot(p.id, removesShot, project.current.updatedAt)
      if (!result.applied) throw new Error('分镜未移除')
      await project.syncFromService(p.id, () => project.current?.id === p.id && !project.dirty)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '移除分镜失败')
      return false
    }
  } else if (n.type === 'prop') {
    const pid = n.data.propId
    if (pid) {
      project.setProps((p.props ?? []).filter((item) => item.id !== pid))
      project.setCanvas(rebuildCanvasGraph(project.current, true))
    } else {
      project.removeCanvasNode(id)
    }
  } else if (n.type === 'subject' || n.type === 'subjectProps') {
    const sid = n.data.subjectId
    if (sid) {
      project.setSubjects((p.subjects ?? []).filter((s) => s.id !== sid))
      project.setShots(
        p.shots.map((shot) => ({
          ...shot,
          subjectIds: (shot.subjectIds ?? []).filter((item) => item !== sid)
        }))
      )
      project.setCanvas(rebuildCanvasGraph(project.current, true))
    } else {
      project.removeCanvasNode(id)
    }
  } else if (n.type === 'scene' || n.type === 'sceneProps') {
    const scid = n.data.sceneId
    if (scid) {
      project.setScenes((p.scenes ?? []).filter((s) => s.id !== scid))
      project.setShots(p.shots.map((shot) => (shot.sceneId === scid ? { ...shot, sceneId: undefined } : shot)))
      project.setCanvas(rebuildCanvasGraph(project.current, true))
    } else {
      project.removeCanvasNode(id)
    }
  } else if (n.type === 'shotImage' || n.type === 'shotBoard' || n.type === 'shotVideo' || n.type === 'shotFrames' || n.type === 'audio') {
    const shotId = n.data.shotId
    if (shotId) {
      project.setTimeline(p.timeline.filter(clip => clip.shotId !== shotId))
      project.setShots(p.shots.filter((shot) => shot.id !== shotId).map((shot, index) => ({ ...shot, index })))
      project.setCanvas(rebuildCanvasGraph(project.current, true))
    } else {
      project.removeCanvasNode(id)
    }
  } else {
    project.removeCanvasNode(id)
  }
  if (selectedId.value === id) selectedId.value = null
  closeCtxMenu()
  if (!options?.quiet) toast.success(removesShot ? '已移除分镜' : '已删除节点')
  return true
}

function keepSpaceForSelection(e: KeyboardEvent) {
  if (e.code !== 'Space' || e.repeat) return
  const target = e.target as HTMLElement | null
  if (!target || target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) return
  e.preventDefault()
}

function onDupKey(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null
  if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'd') {
    e.preventDefault()
    duplicateNode(selectedId.value)
  }
  if (e.key === 'Delete' && canDeleteNode(selectedId.value)) {
    e.preventDefault()
    deleteCtxNode(selectedId.value)
  }
}

function openNodeLinkMenu(nodeId: string, x: number, y: number, side: 'prev' | 'next') {
  const node = project.current?.canvas?.nodes.find((item) => item.id === nodeId)
  const links = side === 'prev' ? prevCanvasLinks(node) : nextCanvasLinks(node)
  if (!links.length) {
    toast.warn(side === 'prev' ? '这个节点前面没有能接上的节点' : '这个节点后面没有能接上的节点')
    return
  }
  selectedId.value = nodeId
  const h = links.length * 36 + 16
  ctxMenu.value = {
    x: Math.max(8, Math.min(x, window.innerWidth - 216)),
    y: Math.max(8, Math.min(y, window.innerHeight - h - 8)),
    flow: screenToFlowCoordinate({ x, y }),
    nodeId,
    onNode: true,
    linkOnly: true,
    linkSide: side,
    links
  }
}

function openCtxMenu(ev: MouseEvent | NodeMouseEvent) {
  const projectId = project.current?.id
  if (webWorkspace && projectId) {
    void listCanvasExtensions().then(items => {
      if (project.current?.id === projectId) extensionTypes.value = items
    }).catch(() => undefined)
  }
  const event = eventFromCtx(ev)
  if (!event) return
  event.preventDefault()
  event.stopPropagation()
  const pt = pointerClient(event)
  if (!pt) return
  const flow = screenToFlowCoordinate({ x: pt.x, y: pt.y })
  const clicked = 'node' in ev && ev.node ? ev.node : undefined
  // 分组框的成员由节点类型自动决定，没有可执行的操作，右键不弹菜单
  if (clicked?.type === 'groupFrame') return
  const clickedId = clicked ? String(clicked.id) : undefined
  if (clickedId) selectedId.value = clickedId
  const nodeId = clickedId || selectedId.value || undefined
  const extra = (canDuplicate(clickedId) ? 36 : 0) + (clickedId && canDeleteNode(clickedId) ? 36 : 0)
  const w = 216
  const h = 8 * 36 + 16 + extra
  ctxMenu.value = {
    x: Math.max(8, Math.min(pt.x, window.innerWidth - w - 8)),
    y: Math.max(8, Math.min(pt.y, window.innerHeight - h - 8)),
    flow,
    nodeId,
    onNode: Boolean(clickedId)
  }
}

function closeCtxMenu() {
  ctxMenu.value = null
}

function pullStrandedFreeformIntoView() {
  const canvas = project.current?.canvas
  if (!canvas) return
  const rescued = rescueStrandedFreeformNodes(canvas.nodes)
  if (!rescued) return
  project.setCanvas({ ...canvas, nodes: rescued })
}

function addFreeform(type: 'imageGen' | 'videoGen' | 'audioGen') {
  if (!project.current) return
  pullStrandedFreeformIntoView()
  const all = project.current.canvas?.nodes ?? []
  const count = all.filter((n) => n.type === type).length + 1
  const id = `${type}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
  const upstream = ctxMenu.value?.linkSide === 'prev'
  const anchorId = ctxMenu.value?.onNode ? ctxMenu.value.nodeId : undefined
  const anchor = anchorId ? all.find((node) => node.id === anchorId) : undefined
  const inheritsSource = !upstream && type !== 'audioGen' && canvasNodeProducesImage(anchor)
  const pos = nextFreeformPosition(all, {
    source: anchor,
    click: ctxMenu.value?.flow,
    place: upstream ? 'before' : 'after'
  })
  project.addCanvasNode({
    id,
    type,
    position: { x: pos.x, y: pos.y },
    data: {
      label: type === 'imageGen' ? `图片 ${count}` : type === 'audioGen' ? `音频 ${count}` : `视频 ${count}`,
      prompt: anchor && project.current ? carryTextFromNode(project.current, anchor) : '',
      referenceAssetIds: [],
      referenceSourceNodeIds: inheritsSource && anchorId ? [anchorId] : [],
      duration: 8,
      aspectRatio: project.current.settings.aspectRatio,
      ...(type === 'videoGen'
        ? {
            videoModel: 'h3' as const,
            videoResolution: project.current.settings.defaultVideoResolution || '768p'
          }
        : {})
    },
    status: 'idle'
  })
  if (upstream && anchorId && type === 'imageGen') {
    appendCanvasReferenceSource(project.current, anchorId, id)
  }
  if (anchorId && project.current.canvas) {
    const source = upstream ? id : anchorId
    const target = upstream ? anchorId : id
    const edge = {
      id: `e-${source}-${target}-in`,
      source,
      target,
      sourceHandle: 'out',
      targetHandle: 'in'
    }
    project.setCanvas({
      ...project.current.canvas,
      edges: [...project.current.canvas.edges.filter((item) => item.id !== edge.id), edge]
    })
  }
  selectedId.value = id
  closeCtxMenu()
  toast.success(type === 'videoGen' ? '已添加视频节点' : type === 'audioGen' ? '已添加音频节点' : '已添加图片节点')
  nextTick(() => {
    fitView({ nodes: [id], padding: 0.35, duration: 280 })
  })
}

function addLinkedText() {
  if (!project.current) return
  const anchorId = ctxMenu.value?.onNode ? ctxMenu.value.nodeId : undefined
  const anchor = anchorId ? project.current.canvas?.nodes.find((node) => node.id === anchorId) : undefined
  const nodes = project.current.canvas?.nodes ?? []
  const pos = anchor
    ? nextFreeformPosition(nodes, { source: anchor, place: 'before', span: 460 })
    : (ctxMenu.value?.flow ?? { x: 120, y: 120 })
  const carried = anchor ? carryTextFromNode(project.current, anchor) : ''
  const script = project.current.script
  if (script && !script.content.trim() && carried) {
    project.setScript({ ...script, content: carried })
  } else if (
    anchor &&
    script?.content.trim() &&
    !carried &&
    (anchor.type === 'imageGen' || anchor.type === 'videoGen' || anchor.type === 'audioGen')
  ) {
    project.setCanvasNodePrompt(anchor.id, script.content.trim().slice(0, 8000))
  }
  const existing = nodes.find((node) => node.type === 'script')
  const scriptId = existing?.id || 'script'
  if (!existing) {
    project.addCanvasNode({
      id: scriptId,
      type: 'script',
      position: pos,
      data: {
        label: project.current.script?.topic || project.current.name || '剧本',
        width: 460,
        height: 720,
        kind: 'scriptDoc'
      },
      status: 'idle'
    })
  } else {
    existing.position = { x: pos.x, y: pos.y }
  }
  if (anchorId && project.current.canvas) {
    const edge = {
      id: `e-${scriptId}-${anchorId}-in`,
      source: scriptId,
      target: anchorId,
      sourceHandle: 'out',
      targetHandle: 'in'
    }
    const linked = project.current.canvas.edges.some((item) => item.source === scriptId && item.target === anchorId)
    project.setCanvas({
      ...project.current.canvas,
      edges: linked ? project.current.canvas.edges : [...project.current.canvas.edges, edge]
    })
  }
  selectedId.value = scriptId
  closeCtxMenu()
  toast.success(existing ? '文本节点已接到前面' : '已在前面添加文本节点')
  nextTick(() => fitView({ nodes: [scriptId], padding: 0.35, duration: 280 }))
}

function addCompose() {
  if (!project.current) return
  const anchorId = ctxMenu.value?.onNode ? ctxMenu.value.nodeId : undefined
  const anchor = anchorId ? project.current.canvas?.nodes.find((node) => node.id === anchorId) : undefined
  const nodes = project.current.canvas?.nodes ?? []
  let compose: CanvasNode | undefined = [...nodes].reverse().find((node) => node.type === 'compose')
  const created = !compose
  if (!compose) {
    const pos = anchor
      ? nextFreeformPosition(nodes, { source: anchor, place: 'after' })
      : (ctxMenu.value?.flow ?? { x: 120, y: 120 })
    const id = `compose_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`
    project.addCanvasNode({
      id,
      type: 'compose',
      position: pos,
      data: { label: '合成' },
      status: 'idle'
    })
    compose = project.current.canvas?.nodes.find(node => node.type === 'compose' && node.id === id)
  }
  if (!compose) return
  if (anchorId && project.current.canvas) {
    const edge = {
      id: `e-${anchorId}-${compose.id}-in`,
      source: anchorId,
      target: compose.id,
      sourceHandle: 'out',
      targetHandle: 'in'
    }
    const linked = project.current.canvas.edges.some((item) => item.source === anchorId && item.target === compose?.id)
    if (!linked) {
      project.setCanvas({
        ...project.current.canvas,
        edges: [...project.current.canvas.edges, edge]
      })
    }
    toast.success(linked ? '这个视频已经在合成节点里' : created ? '已添加合成节点' : '已添加到合成节点')
  }
  selectedId.value = compose.id
  closeCtxMenu()
  const focusId = compose.id
  nextTick(() => fitView({ nodes: [focusId], padding: 0.35, duration: 280 }))
}

function addScriptNode() {
  if (!project.current) return
  const existing = project.current.canvas?.nodes.find((n) => n.type === 'script')
  if (existing) {
    selectedId.value = existing.id
    closeCtxMenu()
    toast.warn('已经有剧本节点')
    return
  }
  const pos = ctxMenu.value?.flow ?? { x: 120, y: 120 }
  project.addCanvasNode({
    id: 'script',
    type: 'script',
    position: pos,
    data: {
      label: project.current.script?.topic || project.current.name || '剧本',
      width: 460,
      height: 720,
      kind: 'scriptDoc'
    },
    status: 'idle'
  })
  selectedId.value = 'script'
  closeCtxMenu()
}

function addPipelineEntity(kind: 'subject' | 'scene' | 'shot' | 'prop') {
  if (!project.current) return
  const flow = ctxMenu.value?.flow ?? { x: 120, y: 120 }
  const before = new Set((project.current.canvas?.nodes ?? []).map((n) => n.id))
  const stamp = Date.now().toString(36)
  let selectId = ''
  if (kind === 'subject') {
    const n = (project.current.subjects ?? []).length + 1
    const id = `sub_${stamp}`
    project.setSubjects([
      ...(project.current.subjects ?? []),
      { id, name: `角色 ${n}`, kind: 'person', description: '' }
    ])
    selectId = `subject:${id}`
  } else if (kind === 'prop') {
    const n = (project.current.props ?? []).length + 1
    const id = `prop_${stamp}`
    project.setProps([
      ...(project.current.props ?? []),
      { id, name: `道具 ${n}`, description: '' }
    ])
    selectId = `prop:${id}`
  } else if (kind === 'scene') {
    const n = (project.current.scenes ?? []).length + 1
    const id = `scn_${stamp}`
    project.setScenes([
      ...(project.current.scenes ?? []),
      { id, name: `场景 ${n}`, description: '' }
    ])
    selectId = `scene:${id}`
  } else {
    const shots = project.current.shots
    const n = shots.length + 1
    const id = `shot_${stamp}`
    const shot: Shot = {
      id,
      index: shots.length,
      title: `镜头 ${n}`,
      visualPrompt: '',
      audioDescription: '',
      voiceover: '',
      duration: project.current.settings.defaultShotDuration || 5,
      cameraMovement: 'static'
    }
    project.setShots([...shots, shot])
    selectId = `shotImage:${id}`
  }
  project.setCanvas(rebuildCanvasGraph(project.current, true))
  const groupId =
    kind === 'subject' ? 'group-subjects' : kind === 'prop' ? 'group-props' : kind === 'scene' ? 'group-scenes' : 'group-shots'
  if (project.current.canvas && !before.has(groupId)) {
    project.setCanvas({
      ...project.current.canvas,
      nodes: project.current.canvas.nodes.map((node) =>
        node.id === groupId ? { ...node, position: flow } : node
      )
    })
  }
  selectedId.value = selectId
  closeCtxMenu()
}

const creatingCanvas = ref(false)
const renaming = ref(false)
const nameInput = ref('')

function startRename() {
  renaming.value = true
  nameInput.value = project.current?.name || ''
}
function commitRename() {
  const name = nameInput.value.trim()
  if (project.current && name) {
    project.current.name = name
    project.current.updatedAt = Date.now()
    project.save()
  }
  renaming.value = false
}

async function newCanvas() {
  if (creatingCanvas.value) return
  creatingCanvas.value = true
  try {
    const now = new Date()
    const pad = (n: number) => String(n).padStart(2, '0')
    const label = `新项目 ${now.getMonth() + 1}-${now.getDate()} ${pad(now.getHours())}:${pad(now.getMinutes())}`
    const p = await project.createNamed(label)
    p.pipelineMode = 'standard'
    p.script = {
      id: 's_' + Date.now().toString(36),
      topic: label,
      content: '',
      keywords: [],
      style: 'cinematic',
      aspectRatio: '16:9',
      shotDuration: 5,
      createdAt: Date.now()
    }
    project.setScript(p.script)
    project.setAspectRatio('16:9')
    await project.save()
    selectedId.value = null
    toast.success('已新建画布')
  } catch (e: any) {
    toast.error(e?.message || '新建画布失败')
  } finally {
    creatingCanvas.value = false
  }
}

async function openFolder() {
  const dir = project.current?.folderPath
  if (dir && isDesktop()) await desktop().openPath(dir)
}

</script>

<template>
  <div class="studio">
    <header class="bar">
      <input
        v-if="renaming"
        v-model="nameInput"
        class="name-input nodrag"
        @keydown.enter="commitRename"
        @keydown.escape="renaming = false"
        @blur="commitRename"
        autofocus
      />
      <button v-else class="project-name nodrag" type="button" title="修改项目名" @click="startRename">
        {{ project.current?.name || '画布' }}
      </button>
      <span class="dim">{{
        `${project.current?.shots.length || 0} 个分镜 · ${project.current?.canvas?.nodes.length || 0} 个节点`
      }}</span>
      <div class="spacer" />
      <ExecutionPlanPanel v-if="webWorkspace && project.current" ref="executionPanel" :project-id="project.current.id" />
      <button class="icon-btn" title="画布使用指南" aria-label="画布使用指南" :aria-expanded="guideOpen" @click="guideOpen = !guideOpen">
        <HomeIcon name="info" :size="17" />
      </button>
      <button
        class="icon-btn"
        v-if="project.current"
        :class="{ on: agentOpen }"
        :title="agentOpen ? '收起连续拍' : '打开连续拍'"
        @click="agentOpen = !agentOpen"
      >
        <CanvasToolbarIcon name="assistant" />
      </button>
      <button class="icon-btn arrange-nodes" type="button" title="整理画布上所有节点" :disabled="arranging || !project.current?.canvas?.nodes.length" @click="relayout">
        <CanvasToolbarIcon name="layout" />
        整理节点
      </button>
      <button
        class="icon-btn"
        :class="{ on: rightOpen }"
        :title="rightOpen ? '收起右侧' : '打开右侧'"
        @click="rightOpen = !rightOpen"
      >
        <CanvasToolbarIcon name="panel" />
      </button>
    </header>
    <StepNav v-if="project.current" horizontal />
    <StudioVideoStatus v-if="project.current" :project-id="project.current.id" />
    <div class="body">
      <CanvasGuide
        v-if="guideOpen"
        :capture-open="agentOpen"
        :capture-width="agentWidth"
        :right-width="rightOpen ? 300 : 0"
        @done="dismissGuide"
      />
      <div
        v-if="project.current && agentOpen"
        class="agent-shell"
        :class="{ resizing: resizingAgent }"
        :style="{ width: `${agentWidth}px` }"
      >
        <ContinuousCaptureControls :selected-node-id="selected?.id" :web-workspace="webWorkspace" @show-plan="executionPanel?.openPlan($event)" />
        <div
          class="agent-resizer"
          role="separator"
          aria-label="调整连续拍面板宽度"
          aria-orientation="vertical"
          :aria-valuemin="AGENT_MIN_WIDTH"
          :aria-valuemax="maxAgentWidth()"
          :aria-valuenow="agentWidth"
          tabindex="0"
          title="左右拖动调整连续拍面板宽度"
          @pointerdown="startAgentResize"
          @keydown.left.prevent="nudgeAgentWidth(-24)"
          @keydown.right.prevent="nudgeAgentWidth(24)"
          @dblclick="agentWidth = 440; persistAgentWidth()"
        />
      </div>
      <div class="flow-wrap">
        <VueFlow
          :nodes="flowNodes"
          :edges="flowEdges"
          :is-valid-connection="connection => Boolean(connection.targetHandle !== 'binding' || canvasBinding(connection.source, connection.target))"
          :node-types="(nodeTypes as any)"
          :default-viewport="project.current?.canvas?.viewport"
          :min-zoom="0.2"
          :max-zoom="2"
          selection-key-code="Space"
          :selection-mode="SelectionMode.Partial"
          :pan-activation-key-code="null"
          fit-view-on-init
          @node-click="onNodeClick"
          @edge-double-click="({ edge }) => edge.targetHandle === 'binding' && changeCanvasBinding(edge.source, edge.target, false)"
          @pane-click="selectionMenu = null; selectionIds = []"
          @selection-end="onBoxSelect"
          @pane-context-menu="openCtxMenu"
          @node-context-menu="openCtxMenu"
        >
          <Controls position="bottom-left" :show-interactive="false" />
        </VueFlow>
        <div v-if="!project.current?.canvas?.nodes.length" class="empty-hint">
          <button v-if="project.current" type="button" class="empty-script" @click="addScriptNode">添加剧本到画布</button>
          <span>也可以在空白处右键添加其他节点。</span>
        </div>
      </div>
      <Teleport to="body">
        <div
          v-if="selectionMenu"
          class="ctx-mask"
          @click="selectionMenu = null"
          @contextmenu.prevent="selectionMenu = null"
        />
        <div
          v-if="selectionMenu"
          class="ctx-menu"
          :style="{ left: `${selectionMenu.x}px`, top: `${selectionMenu.y}px` }"
          @mousedown.stop
        >
          <button
            v-for="action in selectionMenu.actions"
            :key="action.id"
            type="button"
            @mousedown.prevent.stop="runSelectionAction(action.id)"
          >
            {{ action.label }}
          </button>
          <div v-if="selectionMenu.actions.length" class="ctx-sep" />
          <button type="button" class="danger" @mousedown.prevent.stop="deleteSelection">删除</button>
        </div>
        <div
          v-if="ctxMenu"
          class="ctx-mask"
          @click="closeCtxMenu"
          @contextmenu.prevent="openCtxMenu"
        />
        <div
          v-if="ctxMenu"
          class="ctx-menu"
          :style="{ left: `${ctxMenu.x}px`, top: `${ctxMenu.y}px` }"
          @mousedown.stop
        >
          <template v-if="ctxMenu.linkOnly">
            <button
              v-if="ctxMenu.links?.includes('script')"
              type="button"
              @mousedown.prevent.stop="addLinkedText"
            >
              <CtxMenuIcon name="script" />
              文本节点
            </button>
            <button
              v-if="ctxMenu.links?.includes('imageGen')"
              type="button"
              @mousedown.prevent.stop="addFreeform('imageGen')"
            >
              <CtxMenuIcon name="image" />
              图片节点
            </button>
            <button
              v-if="ctxMenu.links?.includes('videoGen')"
              type="button"
              @mousedown.prevent.stop="addFreeform('videoGen')"
            >
              <CtxMenuIcon name="video" />
              视频节点
            </button>
            <button
              v-if="ctxMenu.links?.includes('audioGen')"
              type="button"
              @mousedown.prevent.stop="addFreeform('audioGen')"
            >
              <CtxMenuIcon name="audio" />
              音频节点
            </button>
            <button
              v-if="ctxMenu.links?.includes('compose')"
              type="button"
              @mousedown.prevent.stop="addCompose"
            >
              <CtxMenuIcon name="compose" />
              合成节点
            </button>
          </template>
          <template v-else>
          <button type="button" @mousedown.prevent.stop="addFreeform('imageGen')">
            <CtxMenuIcon name="image" />
            {{ ctxMenu.nodeId ? '追加图片节点' : '添加图片节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addFreeform('videoGen')">
            <CtxMenuIcon name="video" />
            {{ ctxMenu.nodeId ? '追加视频节点' : '添加视频节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addFreeform('audioGen')">
            <CtxMenuIcon name="audio" />
            {{ ctxMenu.nodeId ? '追加音频节点' : '添加音频节点' }}
          </button>
          <button v-for="extension in extensionTypes" :key="extension.id + '@' + extension.version" type="button" @mousedown.prevent.stop="addExtension(extension)">
            {{ extension.name }}
          </button>
          <div class="ctx-sep" />
          <button type="button" @mousedown.prevent.stop="addScriptNode">
            <CtxMenuIcon name="script" />
            {{ ctxMenu.nodeId ? '追加剧本节点' : '添加剧本节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addPipelineEntity('subject')">
            <CtxMenuIcon name="character" />
            {{ ctxMenu.nodeId ? '追加角色节点' : '添加角色节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addPipelineEntity('prop')">
            <CtxMenuIcon name="prop" />
            {{ ctxMenu.nodeId ? '追加道具节点' : '添加道具节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addPipelineEntity('scene')">
            <CtxMenuIcon name="scene" />
            {{ ctxMenu.nodeId ? '追加场景节点' : '添加场景节点' }}
          </button>
          <button type="button" @mousedown.prevent.stop="addPipelineEntity('shot')">
            <CtxMenuIcon name="shot" />
            {{ ctxMenu.nodeId ? '追加分镜节点' : '添加分镜节点' }}
          </button>
          <template v-if="ctxMenu.onNode && (canDuplicate(ctxMenu.nodeId) || canDeleteNode(ctxMenu.nodeId))">
            <div class="ctx-sep" />
            <button
              v-if="canDuplicate(ctxMenu.nodeId)"
              type="button"
              @mousedown.prevent.stop="duplicateNode(ctxMenu.nodeId)"
            >
              <CtxMenuIcon name="duplicate" />
              复制此节点
            </button>
            <button
              v-if="canDeleteNode(ctxMenu.nodeId)"
              type="button"
              class="danger"
              @mousedown.prevent.stop="deleteCtxNode(ctxMenu.nodeId)"
            >
              <CtxMenuIcon name="delete" />
              删除此节点
            </button>
          </template>
          </template>
        </div>
      </Teleport>
      <nav class="side-rail" aria-label="画布工具">
        <button type="button" aria-label="新建画布" title="新建画布" :disabled="creatingCanvas" @click="newCanvas">
          <CanvasToolbarIcon name="newCanvas" :size="18" />
        </button>
        <button
          v-if="isDesktop()"
          type="button"
          aria-label="打开项目文件夹"
          title="打开项目文件夹"
          :disabled="!project.current?.folderPath"
          @click="openFolder"
        >
          <CanvasToolbarIcon name="folder" :size="18" />
        </button>
      </nav>
      <aside v-if="rightOpen" class="dock">
        <div class="dock-title">
          节点
          <button v-if="selected" type="button" class="agent-selection-copy" @click="copySelectedForAgent">复制选区给 Agent</button>
        </div>
        <NodeInspector :node="selected" />
      </aside>
    </div>
  </div>
</template>

<style scoped>
.studio {
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  background: var(--color-bg-0);
  min-width: 0;
}
.bar {
  height: 46px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  border-bottom: 1px solid var(--nb-line-1);
  background: var(--glass-frost);
  font-size: 13px;
  min-width: 0;
}
.project-name, .name-input {
  max-width: 280px;
  border: none;
  background: transparent;
  color: var(--color-text-1);
  font: inherit;
  font-weight: 650;
}
.project-name {
  padding: 4px 6px;
  border-radius: 4px;
  cursor: text;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.project-name:hover { background: var(--control-hover); }
.project-name:active { background: var(--control-pressed); }
.name-input { width: 220px; padding: 4px 6px; border-radius: 6px; background: var(--nb-bg-inset); box-shadow: 0 0 0 1px var(--nb-line-3); }
.bar .dim {
  color: var(--color-text-2);
  font-size: 12px;
  white-space: nowrap;
}
@media (max-width: 640px) {
  .bar .dim { display: none; }
}
.spacer {
  flex: 1;
}
.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  margin: 0;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-2);
  flex-shrink: 0;
  cursor: pointer;
}
.icon-btn svg {
  width: 17px;
  height: 17px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.arrange-nodes {
  width: auto;
  gap: 6px;
  padding: 0 9px;
  white-space: nowrap;
}
.arrange-nodes:disabled {
  opacity: 0.5;
  cursor: default;
}
.icon-btn:hover:not(:disabled) {
  background: var(--control-hover);
  color: var(--color-text-1);
}
.icon-btn.on {
  background: var(--control-selected);
  color: var(--color-text-1);
}
.icon-btn:active:not(:disabled) {
  background: var(--control-pressed);
}
.mode-switch-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 11px;
  border: 1px solid transparent;
  border-radius: 7px;
  background: var(--color-fill);
  color: var(--color-text-2);
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
  cursor: pointer;
  flex-shrink: 0;
  transition: background-color var(--duration-fast) ease, color var(--duration-fast) ease;
}
.mode-switch-btn svg {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.mode-switch-btn em {
  font-style: normal;
  opacity: 0.7;
}
.mode-switch-btn:hover {
  background: var(--control-hover);
  color: var(--color-text-1);
}
.mode-switch-btn:active {
  background: var(--control-pressed);
}
.empty-hint {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  pointer-events: none;
  color: var(--color-text-3);
  font-size: 13px;
}
.empty-script {
  pointer-events: auto;
  padding: 8px 14px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: var(--color-bg-2);
  color: var(--color-text-1);
  cursor: pointer;
}
.empty-script:hover { background: var(--control-hover); }
.empty-script:active { background: var(--control-pressed); }
.ctx-mask {
  position: fixed;
  inset: 0;
  z-index: 9000;
}
.ctx-menu {
  position: fixed;
  z-index: 9001;
  min-width: 200px;
  padding: 6px;
  border-radius: 10px;
  border: 1px solid var(--color-border);
  background: var(--glass-panel);
  box-shadow: var(--glass-elevation);
  -webkit-backdrop-filter: var(--glass-filter);
  backdrop-filter: var(--glass-filter);
}
.ctx-menu button {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin: 0;
  padding: 7px 10px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--color-text-1);
  font-size: 13px;
  text-align: left;
  cursor: pointer;
}
.ctx-menu button svg {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
  color: var(--color-text-3);
}
.ctx-menu button:hover {
  background: var(--control-hover);
}
.ctx-menu button:active {
  background: var(--control-pressed);
}
.ctx-menu button:hover svg {
  color: var(--color-accent);
}
.ctx-menu button.danger {
  color: var(--color-error, #f87171);
}
.ctx-menu button.danger svg {
  color: var(--color-error, #f87171);
}
.ctx-menu button.danger:hover {
  background: rgba(248, 113, 113, 0.12);
}
.ctx-sep {
  height: 1px;
  margin: 4px 6px;
  background: var(--color-border);
}
.dock {
  width: 300px;
  flex-shrink: 0;
  border-left: 1px solid var(--nb-line-1);
  background: var(--color-bg-1);
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.dock-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--nb-line-1);
  flex-shrink: 0;
  font-size: 12px;
  padding: 10px;
  color: var(--color-text-1);
  font-weight: 600;
}
.agent-selection-copy {
  color: var(--color-text-2);
  font: inherit;
  cursor: pointer;
  border-radius: var(--radius-sm);
}
.agent-selection-copy:hover {
  background: var(--control-hover);
  color: var(--color-text-1);
}
.agent-selection-copy:active { background: var(--control-pressed); }
.dock :deep(.insp) {
  width: auto;
  border-left: none;
  flex: 1;
  min-height: 0;
}
.body {
  flex: 1;
  display: flex;
  min-height: 0;
  position: relative;
}
.side-rail {
  width: 46px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 8px 0;
  border-left: 1px solid var(--color-border);
  background: var(--nb-panel-sheen), var(--glass-frost-solid);
  box-shadow: var(--glass-highlight);
  z-index: 5;
}
.side-rail button {
  width: 32px;
  height: 32px;
  margin: 0;
  padding: 0;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-2);
  cursor: pointer;
}
.side-rail button:hover:not(:disabled) {
  background: var(--color-bg-3);
  color: var(--color-text-1);
}
.side-rail button:disabled {
  opacity: 0.35;
  cursor: default;
}
.agent-shell {
  position: relative;
  flex: 0 0 auto;
  min-width: 300px;
  max-width: min(760px, calc(100vw - 360px));
  min-height: 0;
  z-index: 4;
}
.agent-shell :deep(.agent) {
  width: 100%;
  height: 100%;
  border-right: none;
}
.agent-resizer {
  position: absolute;
  top: 0;
  right: -5px;
  bottom: 0;
  width: 10px;
  z-index: 8;
  cursor: col-resize;
  touch-action: none;
  outline: none;
}
.agent-resizer::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: 4px;
  width: 2px;
  background: transparent;
  transition: background 0.15s ease, box-shadow 0.15s ease;
}
.agent-resizer:hover::after,
.agent-resizer:focus-visible::after,
.agent-shell.resizing .agent-resizer::after {
  background: var(--color-accent);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 16%, transparent);
}
.flow-wrap {
  flex: 1;
  min-width: 0;
  position: relative;
}
.flow-wrap :deep(.vue-flow) {
  background: radial-gradient(circle, var(--nb-grid-dot) 0.7px, transparent 1px) 0 0 / 20px 20px
    var(--color-canvas);
}
.flow-wrap :deep(.vue-flow__pane),
.flow-wrap :deep(.vue-flow__viewport) {
  background: transparent;
}
.flow-wrap :deep(.vue-flow__controls) {
  display: flex;
  flex-direction: row;
  gap: 2px;
  margin: 16px;
  padding: 4px;
  border-radius: 14px;
  border: 1px solid var(--nb-line-1);
  background: var(--glass-panel) !important;
  color: var(--nb-t2);
  color-scheme: inherit;
  box-shadow: var(--shadow-sm);
}
.flow-wrap :deep(.vue-flow__controls-button) {
  width: 30px;
  height: 30px;
  padding: 0;
  border: none !important;
  border-bottom: none !important;
  border-radius: 8px;
  background: transparent !important;
  color: var(--nb-t2);
  color-scheme: inherit;
  appearance: none;
  box-sizing: border-box;
}
.flow-wrap :deep(.vue-flow__controls-button svg),
.flow-wrap :deep(.vue-flow__controls-button svg path) {
  max-width: 14px;
  max-height: 14px;
  fill: currentColor !important;
}
.flow-wrap :deep(.vue-flow__controls-button:hover:not(:disabled)) {
  background: var(--control-hover) !important;
  color: var(--nb-t1);
}
.flow-wrap :deep(.vue-flow__controls-button:active:not(:disabled)) {
  background: var(--control-pressed) !important;
}
.flow-wrap :deep(.vue-flow__controls-button:hover svg),
.flow-wrap :deep(.vue-flow__controls-button:hover svg path) {
  fill: currentColor !important;
}
.flow-wrap :deep(.vue-flow__controls-button:disabled) {
  opacity: 0.35;
}
.flow-wrap :deep(.vue-flow__handle) {
  width: 1px !important;
  height: 1px !important;
  min-width: 0 !important;
  min-height: 0 !important;
  background: transparent !important;
  border: none !important;
  opacity: 0 !important;
  pointer-events: none !important;
}
.flow-wrap :deep(.vue-flow__handle.binding-handle) {
  z-index: 2;
  width: 10px !important;
  height: 10px !important;
  background: var(--nb-edge) !important;
  opacity: 1 !important;
  pointer-events: auto !important;
}
.flow-wrap :deep(.vue-flow__pane.selection .vue-flow__node) {
  pointer-events: none;
}
.flow-wrap:has(.vue-flow__node.selected ~ .vue-flow__node.selected) :deep(.vue-flow__node .toolbar-wrap),
.flow-wrap:has(.vue-flow__node.selected ~ .vue-flow__node.selected) :deep(.vue-flow__node .prompt),
.flow-wrap:has(.vue-flow__node.selected ~ .vue-flow__node.selected) :deep(.vue-flow__node .anode .panel) {
  display: none;
}
.flow-wrap :deep(.vue-flow__node:hover:not(.vue-flow__node-groupFrame)) {
  z-index: 12 !important;
}
.flow-wrap :deep(.vue-flow__node) {
  overflow: visible !important;
  padding: 0;
  background: transparent;
  border: none;
  box-shadow: none;
}
.flow-wrap :deep(.vue-flow__node.selected) {
  z-index: 12 !important;
  outline: 1.5px solid var(--focus-ring);
  outline-offset: 1px;
  border-radius: 10px;
}
.flow-wrap :deep(.vue-flow__node-groupFrame) {
  overflow: visible !important;
  margin: 0;
  width: auto;
  height: auto;
}
.flow-wrap :deep(.vue-flow__node-subject),
.flow-wrap :deep(.vue-flow__node-subjectProps),
.flow-wrap :deep(.vue-flow__node-scene),
.flow-wrap :deep(.vue-flow__node-sceneProps),
.flow-wrap :deep(.vue-flow__node-shotBoard),
.flow-wrap :deep(.vue-flow__node-shotImage),
.flow-wrap :deep(.vue-flow__node-shotFrames),
.flow-wrap :deep(.vue-flow__node-shotVideo),
.flow-wrap :deep(.vue-flow__node-audio),
.flow-wrap :deep(.vue-flow__node-export),
.flow-wrap :deep(.vue-flow__node-final),
.flow-wrap :deep(.vue-flow__node-imageGen),
.flow-wrap :deep(.vue-flow__node-videoGen),
.flow-wrap :deep(.vue-flow__node-audioGen),
.flow-wrap :deep(.vue-flow__node-compose) {
  width: auto;
  height: auto;
}
.flow-wrap :deep(.vue-flow__edge-path),
.flow-wrap :deep(.vue-flow__connection-path) {
  stroke: var(--nb-edge) !important;
  stroke-width: 1.8 !important;
  transition: stroke var(--duration-fast) ease, stroke-width var(--duration-fast) ease;
}
.flow-wrap :deep(.vue-flow__edge.pipe-edge:hover),
.flow-wrap :deep(.vue-flow__edge:hover) {
  z-index: 20;
}
.flow-wrap :deep(.vue-flow__edge:hover .vue-flow__edge-path) {
  stroke: var(--nb-accent) !important;
  stroke-width: 2.2 !important;
}
.flow-wrap :deep(.vue-flow__edge .vue-flow__edgeupdater),
.flow-wrap :deep(.vue-flow__edge-interaction) {
  stroke-width: 28px;
  stroke: transparent;
}
.flow-wrap :deep(.vue-flow__connection-line) {
  stroke: var(--nb-accent) !important;
  stroke-width: 1.4 !important;
}
</style>
