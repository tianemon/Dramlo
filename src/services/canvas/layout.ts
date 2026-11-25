import type { CanvasEdge, CanvasGraph, CanvasNode, Project, Shot } from '@/types'
import { normalizeVideoGenModel, stillModeForVideoModel, videoStillNeedsCanvas, type VideoGenModel } from '@/types/canvas'
import { needsExternalTts, shotDisplayTitle } from '@/utils/shot'
import { boundSubjectIds, propsForShot, sceneAssetForShot } from '@/utils/consistency'
import { needsShotFrames, shotUsesFrames, stillModeFromNode, stillTypeForMode } from '@/utils/videoStill'
import { normalizeNodeScale } from '@/utils/nodeResize'
import { canvasNodeOutputAssetId } from './references'
import { useSettingsStore } from '@/stores/settings'

export const NODE_W = 240
const GAP = 18
const PAD = 20
const HEAD = 44
const COL_GAP = 32
const SCRIPT_W = 460
const SCRIPT_H = 720
/** 画面下方名称条高度 */
const NODE_CHROME = 36
/**
 * 流水线自建的 8 个分组。id 固定，`rebuildCanvasGraph` 只认这 8 个；
 * 其余 groupFrame 都是用户手动建的，重整布局时必须原样保留。
 */
export const PIPELINE_GROUP_IDS = [
  'group-subjects',
  'group-props',
  'group-scenes',
  'group-shots',
  'group-frames',
  'group-videos',
  'group-audio',
  'group-export'
] as const
/** 口播节点是独立卡片，不跟分镜画面同一套比例 */
export const AUDIO_NODE_H = 128

function aspectValue(ar?: string): number {
  const [a, b] = String(ar || '16:9').split(':').map(Number)
  if (!a || !b) return 16 / 9
  return a / b
}

/** 画布节点高度：画面区跟片比例走，避免竖屏被裁成横条 */
export function pipelineNodeHeight(aspectRatio?: string): number {
  return NODE_CHROME + Math.round(NODE_W / aspectValue(aspectRatio))
}

/** CSS aspect-ratio，如 16 / 9 */
export function cssAspectRatio(ar?: string): string {
  const [a, b] = String(ar || '16:9').split(':')
  const w = Number(a)
  const h = Number(b)
  if (!w || !h) return '16 / 9'
  return `${w} / ${h}`
}

function groupBox(rows: number, cols: number, nodeH: number) {
  const r = Math.max(1, rows)
  const c = Math.max(1, cols)
  return {
    width: PAD * 2 + c * NODE_W + (c - 1) * GAP,
    height: HEAD + PAD + r * nodeH + Math.max(0, r - 1) * GAP + PAD
  }
}

function cell(row: number, col: number, nodeH: number) {
  return {
    x: PAD + col * (NODE_W + GAP),
    y: HEAD + PAD + row * (nodeH + GAP)
  }
}

type NodeSize = { width: number; height: number }

/** 资产元数据的最小形状，只用于算节点渲染尺寸 */
export type NodeAssetMeta = { kind?: string; width?: number; height?: number }

/**
 * 节点的渲染尺寸。
 *
 * 整理画布只改位置，但网格要按真实宽高留空，否则尾帧、竖图、放大过的
 * 节点会互相压住。界面按钮和智能体的 arrange_canvas 必须共用这一份，
 * 否则两边整理出来的结果不一样。
 */
export function canvasNodeSize(
  project: Project,
  node: CanvasNode,
  getMeta: (id: string) => NodeAssetMeta | undefined
): NodeSize {
  const scale = normalizeNodeScale(node.data.canvasScale)
  const width = node.data.width || (node.type === 'script' ? SCRIPT_W : 260)
  const assetId = node.parentGroupId ? undefined : canvasNodeOutputAssetId(project, node)
  const asset = assetId ? getMeta(assetId) : undefined
  const aspect =
    asset?.kind === 'image' && asset.width && asset.height
      ? `${asset.width}:${asset.height}`
      : node.data.aspectRatio ||
        (node.data.shotId || node.type === 'imageGen' || node.type === 'videoGen'
          ? project.settings.aspectRatio
          : '16:9')
  const height =
    node.data.height ||
    (node.type === 'script'
      ? SCRIPT_H
      : node.type === 'audio' || node.type === 'audioGen'
        ? AUDIO_NODE_H
        : node.type === 'compose'
          ? 182
          : node.type === 'extension' || node.type === 'captureText'
            ? 360
            : pipelineNodeHeight(aspect))
  return { width: Math.ceil(width * scale), height: Math.ceil(height * scale) }
}

/** Arrange the existing graph. Pipeline reconstruction is deliberately a separate operation. */
export function arrangeCanvasNodes(project: Project, measure: (node: CanvasNode) => NodeSize): CanvasGraph {
  const canvas = project.canvas
  const arranged = new Map<string, CanvasNode>()
  const shotOrder = new Map(project.shots.map(shot => [shot.id, shot.index]))
  const groupOrder = PIPELINE_GROUP_IDS as readonly string[]
  const compareId = (a: CanvasNode, b: CanvasNode) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0
  const groupRank = (node: CanvasNode) => {
    const rank = groupOrder.indexOf(node.id)
    return rank === -1 ? groupOrder.length : rank
  }
  const groups = canvas.nodes.filter(node => node.type === 'groupFrame')
    .sort((a, b) => groupRank(a) - groupRank(b) || compareId(a, b))
  const groupIds = new Set(groups.map(group => group.id))
  const originalGroups = new Map<string, string>()
  for (const group of canvas.groups) {
    if (groupIds.has(group.id)) for (const id of group.nodeIds) originalGroups.set(id, group.id)
  }
  const children = new Map<string, CanvasNode[]>()
  const loose: CanvasNode[] = []

  for (const node of canvas.nodes) {
    if (node.type === 'groupFrame') continue
    const parent = node.parentGroupId && groupIds.has(node.parentGroupId) ? node.parentGroupId : originalGroups.get(node.id)
    if (parent) {
      const members = children.get(parent) ?? []
      members.push(parent === node.parentGroupId ? node : { ...node, parentGroupId: parent })
      children.set(parent, members)
    } else {
      loose.push(node)
    }
  }

  function arrangeGrid(nodes: CanvasNode[], columns: number, x: number, y: number): NodeSize {
    const sizes = nodes.map(measure)
    const widths = Array.from({ length: Math.min(columns, nodes.length) }, () => 0)
    sizes.forEach((size, index) => { widths[index % columns] = Math.max(widths[index % columns], size.width) })
    const columnX = widths.map((_, index) => x + widths.slice(0, index).reduce((sum, width) => sum + width + GAP, 0))
    let rowY = y
    for (let offset = 0; offset < nodes.length; offset += columns) {
      const row = nodes.slice(offset, offset + columns)
      row.forEach((node, column) => {
        arranged.set(node.id, { ...node, position: { x: columnX[column], y: rowY } })
      })
      rowY += Math.max(...sizes.slice(offset, offset + columns).map(size => size.height)) + GAP
    }
    return {
      width: widths.reduce((sum, width) => sum + width + GAP, 0) - (widths.length ? GAP : 0),
      height: rowY - y - (nodes.length ? GAP : 0)
    }
  }

  let x = 24
  const scripts = loose.filter(node => node.type === 'script').sort(compareId)
  if (scripts.length) x += arrangeGrid(scripts, 1, x, 24).width + COL_GAP
  for (const group of groups) {
    const members = children.get(group.id) ?? []
    const order = (node: CanvasNode) => shotOrder.get(node.data.shotId ?? '') ?? Number.MAX_SAFE_INTEGER
    members.sort((a, b) => order(a) - order(b) || compareId(a, b))
    const content = arrangeGrid(members, group.id === 'group-export' ? 2 : 1, PAD, HEAD + PAD)
    const width = Math.max(NODE_W, content.width) + 2 * PAD
    const height = HEAD + 2 * PAD + Math.max(64, content.height)
    arranged.set(group.id, { ...group, position: { x, y: 24 }, data: { ...group.data, width, height } })
    x += width + COL_GAP
  }
  const free = loose.filter(node => node.type !== 'script').sort(compareId)
  const neighbors = new Map(free.map(node => [node.id, [] as string[]]))
  const incoming = new Set<string>()
  for (const edge of canvas.edges) {
    if (!neighbors.has(edge.source) || !neighbors.has(edge.target) || edge.source === edge.target) continue
    neighbors.get(edge.source)!.push(edge.target)
    neighbors.get(edge.target)!.push(edge.source)
    incoming.add(edge.target)
  }
  const byId = new Map(free.map(node => [node.id, node]))
  const visited = new Set<string>()
  const isolated: CanvasNode[] = []
  let freeY = 24
  for (const node of [...free.filter(node => !incoming.has(node.id)), ...free]) {
    if (visited.has(node.id)) continue
    const component = [node]
    visited.add(node.id)
    for (const member of component) {
      for (const id of neighbors.get(member.id)!.sort()) {
        if (visited.has(id)) continue
        visited.add(id)
        component.push(byId.get(id)!)
      }
    }
    if (component.length === 1) isolated.push(node)
    else freeY += arrangeGrid(component, 3, x, freeY).height + COL_GAP
  }
  arrangeGrid(isolated, 3, x, freeY)
  return { ...canvas, nodes: canvas.nodes.map(node => arranged.get(node.id) ?? node) }
}

/**
 * 画布节点标题。镜头标题自带镜号（如「01｜0:00-0:03｜…」）时不再加序号，
 * 否则会显示成「1. 01｜0:00-0:03｜…」，同一行出现两个编号。
 */
function shotNodeLabel(shot: Shot): string {
  return shotDisplayTitle(shot)
}

function persistStatus(
  prevStatus: CanvasNode['status'] | undefined,
  hasAsset: boolean
): CanvasNode['status'] {
  // 资产是唯一事实来源：资产已在盘就是已生成，不被磁盘上陈旧的 idle/failed/running 覆盖。
  // 之前的实现只要 prevStatus 非空就原样保留，节点一旦存成 idle 就再也不会因为资产到位而升级。
  if (hasAsset) return 'success'
  // 没有资产时，进程退出后残留的 running/pending 是脏状态，回落 idle；失败信息保留给用户看。
  return prevStatus === 'failed' ? 'failed' : 'idle'
}

function fallbackStillMode() {
  try {
    return stillModeForVideoModel('h3', useSettingsStore().config.video)
  } catch {
    return 'identity' as const
  }
}

function activeVideoProfile() {
  try {
    return useSettingsStore().config.video
  } catch {
    return undefined
  }
}

/** 「创作偏好」里选的默认视频模型（内置工作流）；没选则 undefined，节点照旧走接入默认 */
function defaultVideoModelPref(): VideoGenModel | undefined {
  try {
    return useSettingsStore().config.defaultVideoModel
  } catch {
    return undefined
  }
}

function pos(
  prev: Map<string, CanvasNode>,
  id: string,
  fallback: { x: number; y: number }
) {
  return prev.get(id)?.position ?? fallback
}

/** 分组内节点存的是相对坐标，追加自由节点必须用画布绝对坐标，否则会落到视野外。 */
export function absoluteCanvasPosition(
  nodes: CanvasNode[],
  node: Pick<CanvasNode, 'position' | 'parentGroupId'>
): { x: number; y: number } {
  let x = node.position.x
  let y = node.position.y
  let pid = node.parentGroupId
  const byId = new Map(nodes.map((item) => [item.id, item]))
  const seen = new Set<string>()
  while (pid && !seen.has(pid)) {
    seen.add(pid)
    const parent = byId.get(pid)
    if (!parent) break
    x += parent.position.x
    y += parent.position.y
    pid = parent.parentGroupId
  }
  return { x, y }
}

export function nextFreeformPosition(
  nodes: CanvasNode[],
  opts: {
    source?: CanvasNode
    click?: { x: number; y: number }
    place?: 'before' | 'after'
    span?: number
  }
): { x: number; y: number } {
  let pos: { x: number; y: number }
  if (opts.source) {
    const abs = absoluteCanvasPosition(nodes, opts.source)
    pos =
      opts.place === 'before'
        ? { x: abs.x - (opts.span ?? NODE_W) - 48, y: abs.y }
        : { x: abs.x + NODE_W + 48, y: abs.y }
  } else if (opts.click && Number.isFinite(opts.click.x) && Number.isFinite(opts.click.y)) {
    pos = { x: opts.click.x, y: opts.click.y }
  } else {
    const groups = nodes.filter((n) => n.type === 'groupFrame')
    if (groups.length) {
      const right = Math.max(
        ...groups.map((g) => g.position.x + (Number(g.data.width) || NODE_W))
      )
      pos = { x: right + 48, y: groups[0].position.y + 48 }
    } else {
      pos = { x: 120, y: 120 }
    }
  }
  const taken = nodes.filter((n) => n.type === 'imageGen' || n.type === 'videoGen')
  let guard = 0
  while (
    guard < 24 &&
    taken.some(
      (n) => Math.abs(n.position.x - pos.x) < 36 && Math.abs(n.position.y - pos.y) < 36
    )
  ) {
    pos = opts.place === 'before' ? { x: pos.x - 36, y: pos.y } : { x: pos.x + 36, y: pos.y + 36 }
    guard += 1
  }
  return pos
}

/** 误用分组相对坐标时，节点会堆在画布左上角。拉回分镜视频组右侧。 */
export function rescueStrandedFreeformNodes(nodes: CanvasNode[]): CanvasNode[] | null {
  const groups = nodes.filter((n) => n.type === 'groupFrame')
  const free = nodes.filter(
    (n) => (n.type === 'imageGen' || n.type === 'videoGen') && !n.parentGroupId
  )
  if (!free.length) return null
  const videos = groups.find((g) => g.id === 'group-videos') || groups[groups.length - 1]
  const clusterX = videos ? videos.position.x : Math.max(...groups.map((g) => g.position.x), 0)
  const clusterY = videos ? videos.position.y : Math.min(...groups.map((g) => g.position.y), 0)
  const clusterW = videos ? Number(videos.data.width) || NODE_W : NODE_W
  const stranded = free.filter((n) => {
    if (!groups.length) return false
    return n.position.x + NODE_W < clusterX - 80
  })
  if (!stranded.length) return null
  const startX = clusterX + clusterW + 48
  const startY = clusterY + 48
  const ids = new Set(stranded.map((n) => n.id))
  return nodes.map((n) => {
    if (!ids.has(n.id)) return n
    const i = stranded.findIndex((item) => item.id === n.id)
    return {
      ...n,
      parentGroupId: undefined,
      position: {
        x: startX + (i % 3) * (NODE_W + 36),
        y: startY + Math.floor(i / 3) * 220
      }
    }
  })
}

const FIXED_PIPELINE_TYPES = new Set<CanvasNode['type']>([
  'script',
  'groupFrame',
  'subject',
  'subjectProps',
  'prop',
  'scene',
  'sceneProps',
  'shotBoard',
  'shotImage',
  'shotFrames',
  'shotVideo',
  'audio',
  'export',
  'final'
])

export function projectShotBindingEdges(project: Project, graph: CanvasGraph): CanvasEdge[] {
  const assets = new Map<string, CanvasNode>()
  const targets = new Map<string, CanvasNode[]>()
  const byId = new Map(graph.nodes.map(node => [node.id, node]))
  for (const node of graph.nodes) {
    if (node.type === 'subject' && node.data.subjectId) assets.set(`subject:${node.data.subjectId}`, node)
    if (node.type === 'scene' && node.data.sceneId) assets.set(`scene:${node.data.sceneId}`, node)
    if (node.type === 'prop' && node.data.propId) assets.set(`prop:${node.data.propId}`, node)
    if (!node.data.shotId || node.data.captureId || !['shotBoard', 'shotImage', 'shotFrames', 'shotVideo'].includes(node.type)) continue
    const nodes = targets.get(node.data.shotId) ?? []
    nodes.push(node)
    targets.set(node.data.shotId, nodes)
  }
  const edges = graph.edges.filter(edge => {
    const source = byId.get(edge.source)
    const target = byId.get(edge.target)
    return !(source && target?.data.shotId && !target.data.captureId &&
      ['subject', 'scene', 'prop'].includes(source.type) &&
      ['shotBoard', 'shotImage', 'shotFrames', 'shotVideo'].includes(target.type) &&
      !['first-frame', 'last-frame'].includes(edge.targetHandle ?? ''))
  })
  for (const shot of project.shots) {
    const nodes = targets.get(shot.id) ?? []
    const stills = nodes.filter(node => node.type !== 'shotVideo')
    const sources = [
      ...boundSubjectIds(shot, project.subjects ?? []).map(id => `subject:${id}`),
      ...propsForShot(shot, project).map(prop => `prop:${prop.id}`)
    ]
    const scene = sceneAssetForShot(shot, project)
    if (scene) sources.push(`scene:${scene.id}`)
    for (const key of sources) {
      const source = assets.get(key)
      if (!source) continue
      for (const target of stills.length ? stills : nodes) {
        edges.push({ id: `binding:${source.id}:${target.id}`, source: source.id, target: target.id,
          sourceHandle: 'out', targetHandle: 'binding', deletable: false })
      }
    }
  }
  return edges
}

export function rebuildCanvasGraph(project: Project, preservePositions = true): CanvasGraph {
  const prevList = project.canvas.nodes
  if (project.continuousCaptureEnabled) {
    const nodes = prevList.filter((node) => !FIXED_PIPELINE_TYPES.has(node.type))
    const ids = new Set(nodes.map((node) => node.id))
    return {
      nodes,
      edges: project.canvas.edges.filter((edge) => ids.has(edge.source) && ids.has(edge.target)),
      groups: [],
      viewport: project.canvas.viewport
    }
  }
  const subjects = project.subjects ?? []
  const storyProps = project.props ?? []
  const scenes = project.scenes ?? []
  const shots = project.shots
  const hasPipelineData = subjects.length > 0 || storyProps.length > 0 || scenes.length > 0 || shots.length > 0
  // 还没进流水线时只铺用户手动加的节点，不铺空的角色/场景/分镜分组
  const bare = !hasPipelineData
  const hasScriptNode = prevList.some((n) => n.id === 'script' || n.type === 'script')
  // 既没有流水线数据也没有手动加的节点时，保持空画布（由智能体或右键添加来起头）
  if (!hasPipelineData && !hasScriptNode && !prevList.length) {
    return project.canvas
  }
  const hadGroups = prevList.some((n) => n.type === 'groupFrame')
  const shotVideoPrev = prevList.filter((n) => n.type === 'shotVideo')
  const defaultStill = fallbackStillMode()
  // 走首尾帧的镜头视频直接吃起始帧/结束帧，不再铺关键帧/看板节点；
  // 已经画出来的关键帧保留在画布上（不藏用户的图），只是不再要求补。
  const shotsNeedingStills = shots.filter(
    (s) => !shotUsesFrames(s, project) || s.imageAssetId || s.boardAssetId
  )
  const stillsWanted =
    shots.length > 0
      ? shotsNeedingStills.length > 0
      : (project.settings?.h3ShotChainMode || 'auto') !== 'fl2va'
  const useStills =
    shots.some(s => s.imageAssetId || s.boardAssetId) ||
    (stillsWanted &&
    (shotVideoPrev.length
      ? shotVideoPrev.some((n) => videoStillNeedsCanvas(stillModeFromNode(n)))
      : videoStillNeedsCanvas(defaultStill)))
  const stillGroupLabel = shotVideoPrev.some((n) => stillModeFromNode(n) === 'storyboard')
    ? '故事看板'
    : '分镜关键帧'
  const videosHomed =
    shots.length === 0 ||
    !prevList.some((n) => n.type === 'shotVideo') ||
    prevList
      .filter((n) => n.type === 'shotVideo')
      .every((n) => n.parentGroupId === 'group-videos')
  // 首尾帧组只在真的要用首尾帧出片时出现：项目开了首尾帧链，或有镜头手动选了首尾帧。
  // 它是给视频供图的，所以排在分镜关键帧之后、分镜视频之前。
  // 口径与「下一步」流水线、智能体判断共用 needsShotFrames，三边必须一致。
  const needFrames = shots.length > 0 && needsShotFrames(project)
  const audioNeeded = (shot: typeof shots[number]) => !bare && (Boolean(shot.audioAssetId) || needsExternalTts(shot, project))
  const needAudio = shots.some(audioNeeded)
  // 首次插入这一组时，后面的分组要整体右移，旧坐标会跟新分组撞在一起，只能重排一次。
  const framesJustAdded = needFrames && !prevList.some((n) => n.id === 'group-frames')
  const layoutReady = bare
    ? shots.length === 0 || (prevList.some((n) => n.id === 'group-videos') && videosHomed)
    : hadGroups &&
      (!needAudio || prevList.some((n) => n.id === 'group-audio')) &&
      prevList.some((n) => n.id === 'group-videos') &&
      videosHomed &&
      (shots.length === 0 ||
        !useStills ||
        prevList.some((n) => n.type === 'shotBoard' || n.type === 'shotImage'))
  const keepPositions = preservePositions && !framesJustAdded
  const prev = new Map((keepPositions && layoutReady ? prevList : []).map((n) => [n.id, n]))
  const prevAny = new Map(prevList.map((n) => [n.id, n]))
  const nodes: CanvasNode[] = []
  const edges: CanvasEdge[] = []
  const groups: CanvasGraph['groups'] = []
  const shotH = pipelineNodeHeight(project.settings?.aspectRatio)
  const sheetH = pipelineNodeHeight('16:9')

  const prevScript = prevAny.get('script')
  const scriptW = prevScript?.data.width || SCRIPT_W
  const scriptH = prevScript?.data.height || SCRIPT_H
  const keepScript = !bare || hasScriptNode
  const scriptPos =
    prevScript && !prevScript.parentGroupId
      ? prevScript.position
      : { x: 24, y: 24 }

  if (keepScript) {
    nodes.push({
      id: 'script',
      type: 'script',
      position: scriptPos,
      data: {
        label: project.script?.topic || project.name || '剧本',
        width: scriptW,
        height: scriptH,
        kind: 'scriptDoc'
      },
      status: persistStatus(prevScript?.status, Boolean(project.script?.content))
    })
  }

  const subBox = groupBox(subjects.length, 1, sheetH)
  const propBox = groupBox(storyProps.length, 1, sheetH)
  const scnBox = groupBox(scenes.length, 1, sheetH)
  const shotBox = groupBox(shots.length, 1, shotH)
  // 首尾帧列只需覆盖到最后一个真的走首尾帧的镜头，不必按全片镜数撑满整列。
  const frameRows = shots.reduce((max, shot, i) => shotUsesFrames(shot, project) ? i + 1 : max, 0)
  const frameBox = groupBox(Math.max(frameRows, 1), 1, shotH)
  const vidBox = groupBox(shots.length, 1, shotH)
  const audBox = groupBox(shots.length, 1, AUDIO_NODE_H)
  const expBox = groupBox(1, 2, shotH)

  let x = (keepScript ? scriptPos.x + scriptW : 24) + COL_GAP
  const y = keepScript ? scriptPos.y : 24

  function pushGroup(
    id: string,
    label: string,
    kind: string,
    box: { width: number; height: number }
  ) {
    nodes.push({
      id,
      type: 'groupFrame',
      position: pos(prev, id, { x, y }),
      data: {
        label, kind,
        // 分组框没有手动 resize 入口，尺寸就是当前内容的精确值；
        // 之前取 Math.max(旧值) 会让框永远停在历史最大尺寸，删镜或改模式后收不回去。
        width: box.width,
        height: box.height
      },
      status: 'idle'
    })
    groups.push({ id, label, nodeIds: [] })
    x += box.width + COL_GAP
  }

  if (!bare || subjects.length) pushGroup('group-subjects', '人物角色', 'subject', subBox)
  if (storyProps.length) pushGroup('group-props', '反复出现的道具', 'prop', propBox)
  if (!bare || scenes.length) pushGroup('group-scenes', '场景锚点', 'scene', scnBox)
  if ((!bare || shots.length) && useStills) {
    pushGroup('group-shots', stillGroupLabel, 'shot', shotBox)
  }
  if ((!bare || shots.length) && needFrames) {
    pushGroup('group-frames', '首尾帧', 'frames', frameBox)
  }
  if (!bare || shots.length) {
    pushGroup('group-videos', '分镜视频', 'video', vidBox)
  }
  if (needAudio) pushGroup('group-audio', '口播', 'audio', audBox)
  if (!bare) pushGroup('group-export', '合成视频', 'export', expBox)

  function attach(groupId: string, node: CanvasNode) {
    if (!nodes.some((item) => item.id === groupId)) return
    const old = prevAny.get(node.id)
    const sameParent = old?.parentGroupId === groupId
    if (
      keepPositions &&
      sameParent === false &&
      old &&
      old.parentGroupId === undefined &&
      node.type !== 'shotVideo' &&
      prevList.some((n) => n.id === node.id)
    ) {
      nodes.push({ ...node, position: old.position, parentGroupId: undefined })
      return
    }
    node.parentGroupId = groupId
    // 换组后不能沿用旧组里的相对坐标，否则会落到框外（例如从分镜第 2 列迁到分镜视频）
    node.position = sameParent ? pos(prev, node.id, node.position) : node.position
    nodes.push(node)
    const g = groups.find((x) => x.id === groupId)
    g?.nodeIds.push(node.id)
  }

  subjects.forEach((s, i) => {
    const id = `subject:${s.id}`
    const previous = prevAny.get(id)
    attach('group-subjects', {
      id,
      type: 'subject',
      position: cell(i, 0, sheetH),
      data: { subjectId: s.id, label: s.name, width: previous?.data.width, height: previous?.data.height,
        aspectRatio: previous?.data.aspectRatio, imageResolution: previous?.data.imageResolution },
      status: persistStatus(prev.get(id)?.status, Boolean(s.designSheetAssetId)),
      parentGroupId: 'group-subjects'
    })
    if (keepScript) {
      edges.push({
        id: `e-script-${id}`,
        source: 'script',
        target: id,
        sourceHandle: 'out',
        targetHandle: 'in'
      })
    }
  })

  storyProps.forEach((item, i) => {
    const id = `prop:${item.id}`
    attach('group-props', {
      id,
      type: 'prop',
      position: cell(i, 0, sheetH),
      data: { propId: item.id, label: item.name },
      status: persistStatus(prev.get(id)?.status, Boolean(item.designSheetAssetId)),
      parentGroupId: 'group-props'
    })
    if (keepScript) {
      edges.push({
        id: `e-script-${id}`,
        source: 'script',
        target: id,
        sourceHandle: 'out',
        targetHandle: 'in'
      })
    }
  })

  scenes.forEach((s, i) => {
    const id = `scene:${s.id}`
    attach('group-scenes', {
      id,
      type: 'scene',
      position: cell(i, 0, sheetH),
      data: { sceneId: s.id, label: s.name },
      status: persistStatus(prev.get(id)?.status, Boolean(s.plateAssetId)),
      parentGroupId: 'group-scenes'
    })
    if (keepScript) {
      edges.push({
        id: `e-script-${id}`,
        source: 'script',
        target: id,
        sourceHandle: 'out',
        targetHandle: 'in'
      })
    }
  })

  shots.forEach((shot, i) => {
    const vidId = `shotVideo:${shot.id}`
    const audId = `audio:${shot.id}`
    const prevVid = prev.get(vidId) || prevAny.get(vidId)
    const model = normalizeVideoGenModel(prevVid?.data.videoModel ?? defaultVideoModelPref())
    const resolvedStill =
      prevVid?.data.videoStillMode === 'identity' ||
      prevVid?.data.videoStillMode === 'keyframe' ||
      prevVid?.data.videoStillMode === 'storyboard'
        ? prevVid.data.videoStillMode
        : stillModeForVideoModel(model, activeVideoProfile())
    // 首尾帧镜头不铺关键帧节点；但这镜如果已经画过关键帧/看板，保留节点不删图
    const keepExistingStill = Boolean(shot.imageAssetId || shot.boardAssetId)
    const stillType =
      shotUsesFrames(shot, project) && !keepExistingStill ? null :
        stillTypeForMode(resolvedStill) || (shot.imageAssetId ? 'shotImage' : shot.boardAssetId ? 'shotBoard' : null)
    const stillId = stillType ? `${stillType}:${shot.id}` : ''
    if (stillType && stillId) {
      const prevStillNode = prev.get(stillId) || prevAny.get(stillId)
      attach('group-shots', {
        id: stillId,
        type: stillType,
        position: cell(i, 0, shotH),
        data: {
          shotId: shot.id,
          label: shotNodeLabel(shot),
          imageResolution: prevStillNode?.data.imageResolution,
          aspectRatio: prevStillNode?.data.aspectRatio ?? project.settings.aspectRatio
        },
        status: persistStatus(
          prev.get(stillId)?.status,
          Boolean(stillType === 'shotBoard' ? shot.boardAssetId : shot.imageAssetId)
        ),
        parentGroupId: 'group-shots'
      })
    }
    // 首尾帧节点只铺给真正走首尾帧的镜头：别的镜头用参考图，不该被项目里某一镜的设置带着一起铺。
    if (shotUsesFrames(shot, project)) {
      const frameId = `shotFrames:${shot.id}`
      const prevFrame = prev.get(frameId) || prevAny.get(frameId)
      attach('group-frames', {
        id: frameId,
        type: 'shotFrames',
        position: cell(i, 0, shotH),
        data: {
          shotId: shot.id,
          label: shotNodeLabel(shot),
          imageResolution: prevFrame?.data.imageResolution,
          aspectRatio: prevFrame?.data.aspectRatio ?? project.settings.aspectRatio
        },
        status: prevFrame?.status === 'failed' && prevFrame.error?.trim() ? 'failed'
          : shot.firstFrameAssetId && shot.lastFrameAssetId ? 'success' : 'idle',
        error: prevFrame?.status === 'failed' ? prevFrame.error?.trim() || undefined : undefined,
        parentGroupId: 'group-frames'
      })
    }
    attach('group-videos', {
      id: vidId,
      type: 'shotVideo',
      position: cell(i, 0, shotH),
      data: {
        shotId: shot.id,
        label: shotNodeLabel(shot),
        videoModel: prevVid?.data.videoModel ?? defaultVideoModelPref(),
        width: prevVid?.data.width,
        height: prevVid?.data.height,
        // 只在节点确实设过时才写；没设过就留空，读取侧回退项目默认。
        // 以前这里把 settings 的默认值抄进每个节点，导致改项目默认画幅/分辨率对这些节点不生效。
        videoResolution: prevVid?.data.videoResolution,
        videoStillMode: prevVid?.data.videoStillMode,
        generateAudio: prevVid?.data.generateAudio,
        aspectRatio: prevVid?.data.aspectRatio
      },
      status: persistStatus(prevVid?.status, Boolean(shot.videoAssetId)),
      parentGroupId: 'group-videos'
    })
    if (audioNeeded(shot)) {
      const prevAudio = prev.get(audId) || prevAny.get(audId)
      attach('group-audio', {
        id: audId,
        type: 'audio',
        position: cell(i, 0, AUDIO_NODE_H),
        data: {
          shotId: shot.id,
          ttsModel: prevAudio?.data.ttsModel,
          clonedVoiceId: prevAudio?.data.clonedVoiceId,
          referenceAudioAssetId: prevAudio?.data.referenceAudioAssetId,
          // label 统一是节点标题：声音类型由 AudioNode 从 shot.audioMode 实时派生，不塞进 label。
          label: shotNodeLabel(shot)
        },
        status: persistStatus(
          prev.get(audId)?.status,
          Boolean(shot.audioAssetId) || !needsExternalTts(shot, project)
        ),
        parentGroupId: 'group-audio'
      })
    }

    const frameId = shotUsesFrames(shot, project) ? `shotFrames:${shot.id}` : ''
    if (frameId) {
      edges.push({
        id: `e-${frameId}-${vidId}`,
        source: frameId,
        target: vidId,
        sourceHandle: 'out',
        targetHandle: 'frames'
      })
    }
    if (stillType && stillId) {
      edges.push({
        id: `e-${stillId}-${vidId}`,
        source: stillId,
        target: vidId,
        sourceHandle: 'out',
        targetHandle: 'image'
      })
    }
    if (!bare) {
      if (audioNeeded(shot)) edges.push({
        id: `e-${audId}-export`,
        source: audId,
        target: 'export',
        sourceHandle: 'out',
        targetHandle: 'in'
      })
      edges.push({
        id: `e-${vidId}-export`,
        source: vidId,
        target: 'export',
        sourceHandle: 'out',
        targetHandle: 'in'
      })
    }
  })

  if (!bare) {
    const hasFinal = Boolean(project.finalVideoAssetId)
    attach('group-export', {
      id: 'export',
      type: 'export',
      position: cell(0, 0, shotH),
      data: { label: '合成视频' },
      status: persistStatus(prev.get('export')?.status, hasFinal),
      parentGroupId: 'group-export'
    })
    attach('group-export', {
      id: 'final',
      type: 'final',
      position: cell(0, 1, shotH),
      data: { label: '视频预览' },
      status: persistStatus(prev.get('final')?.status, hasFinal),
      parentGroupId: 'group-export'
    })
    edges.push({
      id: 'e-export-final',
      source: 'export',
      target: 'final',
      sourceHandle: 'out',
      targetHandle: 'in'
    })
  }

  // 用户手动建的分组不在流水线模板里，重整布局时整组保留（框 + 成员归属）。
  // 不保留的话，成员会被清掉 parentGroupId，而它们的坐标是相对分组的，
  // 回到绝对坐标后就会漂到别处。
  const pipelineGroupIds = new Set<string>(PIPELINE_GROUP_IDS)
  const customGroups = prevList.filter(
    (node) => node.type === 'groupFrame' && !pipelineGroupIds.has(node.id)
  )
  for (const group of customGroups) {
    if (nodes.some((item) => item.id === group.id)) continue
    nodes.push({ ...group })
    groups.push({
      id: group.id,
      label: String(group.data?.label ?? '分组'),
      nodeIds: prevList
        .filter((node) => node.parentGroupId === group.id)
        .map((node) => node.id)
    })
  }
  const customGroupIds = new Set(customGroups.map((group) => group.id))

  // 固定流程是模板层；用户追加的自由生图/视频节点属于创作层，重整布局时不能覆盖。
  const freeNodes = prevList.filter(
    (node) => Boolean(node.data.captureId) || node.type === 'imageGen' || node.type === 'videoGen' || node.type === 'audioGen' || node.type === 'compose' || node.type === 'extension'
  )
  const freeIds = new Set(freeNodes.map((node) => node.id))
  for (const node of freeNodes) {
    if (nodes.some((item) => item.id === node.id)) continue
    const parent = node.parentGroupId && customGroupIds.has(node.parentGroupId) ? node.parentGroupId : undefined
    nodes.push({ ...node, parentGroupId: parent })
    if (parent) {
      const group = groups.find((item) => item.id === parent)
      if (group) group.nodeIds.push(node.id)
    }
  }
  const nodeIds = new Set(nodes.map((node) => node.id))
  for (const edge of project.canvas.edges) {
    if (!freeIds.has(edge.source) && !freeIds.has(edge.target)) continue
    if (!nodeIds.has(edge.source) || !nodeIds.has(edge.target)) continue
    if (!edges.some((item) => item.id === edge.id)) edges.push(edge)
  }

  const viewport = project.canvas.viewport
  for (const node of nodes) {
    const scale = prevAny.get(node.id)?.data.canvasScale
    if (scale !== undefined) node.data.canvasScale = scale
  }
  // 人物/场景/道具连到镜头的线是「镜头字段的投影」，删掉它并不会改变绑定，
  // 所以标成不可删，避免出现「删了线、生成时人物还在」的错觉。要解绑请在属性面板改。
  for (const edge of edges) {
    if (edge.source.startsWith('subject:') || edge.source.startsWith('scene:') || edge.source.startsWith('prop:')) {
      edge.deletable = false
    }
  }
  const graph = { nodes, edges, groups, viewport }
  return { ...graph, edges: projectShotBindingEdges(project, graph) }
}
