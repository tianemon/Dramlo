/**
 * 项目 store
 *
 * - 持有当前项目
 * - 自动持久化到 IndexedDB
 * - 提供新建/加载/保存/删除
 */

import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'
import {
  saveProject,
  loadProject,
  listProjects,
  deleteProject,
  setActiveProjectDir,
  loadProjectFromDir
} from '@/services/storage'
import { isDesktop, desktop } from '@/services/desktop'
import { acceptsCanvasData, emptyCanvas, normalizeH3GenMode, normalizeVideoGenModel, type CanvasNodeData } from '@/types/canvas'
import projectDefaults from '@/types/projectDefaults.json'
import { needsExternalTts, pushHistory, shotDisplayTitle } from '@/utils/shot'
import { projectShotBindingEdges, rebuildCanvasGraph } from '@/services/canvas/layout'
import { callWorkspace } from '@/services/localWebStorage'
import { removeCanvasNodeReference } from '@/services/canvas/references'
import { aspectToSize } from '@/utils/aspect'
import { boundSubjectIds, propsForShot } from '@/utils/consistency'
import type {
  Project,
  Script,
  ScriptChapter,
  Shot,
  TimelineClip,
  ProjectSettings,
  AgentWorkspaceState,
  AspectRatio,
  VideoResolution,
  ID,
  Subject,
  StoryProp,
  Scene,
  CanvasGraph,
  CanvasNode,
  ShotAudioMode
} from '@/types'

const STORAGE_KEY_LAST = 'dramlo:last-project-id'

function newProject(): Project {
  const now = Date.now()
  return {
    id: 'p_' + now.toString(36),
    name: '未命名项目',
    script: null,
    shots: [],
    subjects: [],
    scenes: [],
    timeline: [],
    settings: structuredClone(projectDefaults as ProjectSettings),
    canvas: emptyCanvas(),
    agentState: {
      autonomy: 'gated',
      messages: [],
      tasks: [],
      callBudget: 24
    },
    createdAt: now,
    updatedAt: now
  }
}

/**
 * 画布节点的 label 是从镜头派生的持久字段，又有自己的消费者：
 * 智能体按它匹配节点、参考图用它做名称、导出用它做文件名。
 * 镜头标题或顺序一变就必须同步，否则界面上看到新标题，下游仍在用旧标题。
 */
function syncShotNodeLabels(project: Project | null) {
  const canvas = project?.canvas
  if (!project || !canvas) return
  const shots = new Map(project.shots.map((shot) => [shot.id, shot]))
  for (const node of canvas.nodes) {
    if (!['shotImage', 'shotBoard', 'shotFrames', 'shotVideo', 'audio'].includes(node.type)) continue
    const shot = node.data.shotId ? shots.get(node.data.shotId) : undefined
    if (!shot) continue
    const label = shotDisplayTitle(shot)
    if (node.data.label !== label) node.data = { ...node.data, label }
  }
}

/** 资产一变就同步节点状态：节点状态只是资产的展示，不该等下次进画布才更新。 */
function syncShotNodeStatus(project: Project | null) {
  const canvas = project?.canvas
  if (!project || !canvas) return
  const shots = new Map(project.shots.map((shot) => [shot.id, shot]))
  for (const node of canvas.nodes) {
    // 生成中的节点由任务自己管状态，不在这里覆盖
    if (node.status === 'running' || node.status === 'pending') continue
    const shot = node.data.shotId ? shots.get(node.data.shotId) : undefined
    if (!shot) continue
    let has = false
    if (node.type === 'shotImage') has = Boolean(shot.imageAssetId)
    else if (node.type === 'shotBoard') has = Boolean(shot.boardAssetId)
    else if (node.type === 'shotVideo') has = Boolean(shot.videoAssetId)
    else if (node.type === 'audio') has = Boolean(shot.audioAssetId) || !needsExternalTts(shot, project)
    else if (node.type === 'shotFrames') has = Boolean(shot.firstFrameAssetId && shot.lastFrameAssetId)
    else continue
    const next = has ? 'success' : node.status === 'failed' ? 'failed' : 'idle'
    if (node.status !== next) node.status = next
  }
}

export const useProjectStore = defineStore('project', () => {
  const current = ref<Project | null>(null)
  const allProjects = ref<Project[]>([])
  const dirty = ref(false)
  const lastSavedAt = ref<number | null>(null)
  /** 首次从 IndexedDB 恢复完成，视图据此避免「还没加载就当成空项目」 */
  const hydrated = ref(false)
  let hydratePromise: Promise<void> | null = null

  const isLoaded = computed(() => current.value !== null)
  const shotCount = computed(() => current.value?.shots.length ?? 0)
  const totalDuration = computed(() =>
    (current.value?.timeline ?? []).reduce((sum, c) => sum + c.duration, 0)
  )

  function rememberCurrent() {
    if (current.value) {
      localStorage.setItem(STORAGE_KEY_LAST, current.value.id)
    }
  }

  /** 加载项目列表 */
  let activeProjectLocked = false
  let projectTransition = false
  const removingProjects = new Set<string>()
  let removalVersion = 0

  function lockActiveProject() {
    if (activeProjectLocked || projectTransition) throw new Error('工作台正在切换项目或执行外部任务')
    activeProjectLocked = true
    return () => { activeProjectLocked = false }
  }

  function assertProjectUnlocked() {
    if (activeProjectLocked) throw new Error('请等待当前外部任务完成后再切换项目')
  }

  async function changeProject<T>(run: () => Promise<T>) {
    assertProjectUnlocked()
    if (projectTransition) throw new Error('工作台正在切换项目')
    projectTransition = true
    try {
      return await run()
    } finally {
      projectTransition = false
    }
  }

  let listRequest = 0
  async function refreshList() {
    const request = ++listRequest
    const projects = await listProjects()
    if (request === listRequest) allProjects.value = projects
    return projects
  }

  /**
   * 路由进入工作流页之前调用：从 IDB 恢复上次项目，避免刷新后每一步都像没做过。
   * 不自动新建空项目（首页自己处理）。
   */
  async function ensureLoaded() {
    if (hydrated.value) return
    if (hydratePromise) return hydratePromise
    hydratePromise = (async () => {
      await refreshList()
      if (!current.value) {
        const lastId = localStorage.getItem(STORAGE_KEY_LAST)
        const target =
          (lastId && allProjects.value.find((p) => p.id === lastId)) ||
          allProjects.value[0]
        if (target) {
          await load(target.id)
        }
      }
      hydrated.value = true
    })().finally(() => {
      hydratePromise = null
    })
    return hydratePromise
  }

  /** 加载项目 */
  async function load(id: string, shouldApply?: () => boolean) {
    if (removingProjects.has(id)) return
    const before = removalVersion
    const canApply = () => before === removalVersion && !removingProjects.has(id) && (shouldApply?.() ?? true)
    const p = await loadProject(id, shouldApply ? canApply : undefined)
    if (p && canApply()) activateProject(p)
  }

  function activateProject(p: Project) {
    if (p.canvas?.nodes.length) {
      for (const node of p.canvas.nodes) {
        if (node.status === 'running' || node.status === 'pending') {
          node.status = 'idle'
          delete node.error
        }
      }
      syncShotNodeLabels(p)
      syncShotNodeStatus(p)
      p.canvas.edges = projectShotBindingEdges(p, p.canvas)
    }
    for (const task of p.agentState?.tasks ?? []) {
      if (task.status === 'running') {
        task.status = 'cancelled'
        task.finishedAt = Date.now()
      }
    }
    if (p.folderPath) setActiveProjectDir(p.folderPath)
    current.value = p
    dirty.value = false
    rememberCurrent()
  }

  /** 新建项目 */
  function create(name = '未命名项目'): Project {
    assertProjectUnlocked()
    if (current.value && dirty.value) {
      saveProject(current.value).catch(() => {})
    }
    const p = newProject()
    p.name = name
    current.value = p
    dirty.value = true
    rememberCurrent()
    return p
  }

  async function createNamed(name: string): Promise<Project> {
    const p = create(name)
    if (isDesktop()) {
      const dir = await desktop().ensureProjectDir(p.name, p.id)
      p.folderPath = dir
      setActiveProjectDir(dir)
      await save()
      await desktop().setActiveProject?.(p.id)
    }
    return p
  }

  async function openFromFolder(dir: string) {
    const p = await loadProjectFromDir(dir)
    if (!p) return
    activateProject(p)
    await refreshList()
  }

  function rename(name: string) {
    if (!current.value) return
    current.value.name = name
    touch()
  }

  /** 设置文案 */
  function setScript(script: Script) {
    if (!current.value) return
    current.value.script = script
    touch()
  }

  function setChapters(chapters: ScriptChapter[]) {
    if (!current.value) return
    current.value.chapters = chapters.map((chapter, index) => ({ ...chapter, order: index }))
    const ids = new Set(current.value.chapters.map((chapter) => chapter.id))
    current.value.selectedChapterIds = (current.value.selectedChapterIds ?? []).filter((id) => ids.has(id))
    current.value.productionChapterIds = (current.value.productionChapterIds ?? []).filter((id) => ids.has(id))
    touch()
  }

  function setSelectedChapterIds(ids: ID[]) {
    if (!current.value) return
    const existing = new Set((current.value.chapters ?? []).map((chapter) => chapter.id))
    current.value.selectedChapterIds = [...new Set(ids)].filter((id) => existing.has(id))
    touch()
  }

  function setProductionChapterIds(ids: ID[]) {
    if (!current.value) return
    current.value.productionChapterIds = [...new Set(ids)]
    touch()
  }

  /** 用剧本名称作为项目标题 */
  function applyPlayTitle(title?: string) {
    if (!current.value) return
    const name = (title || '').replace(/\s+/g, ' ').trim().slice(0, 48)
    if (!name || current.value.name === name) return
    current.value.name = name
    touch()
  }

  /** 设置分镜 */
  function setShots(shots: Shot[]) {
    if (!current.value) return
    current.value.shots = shots
    // 整批替换也发生在导入、重排、删除之后；不同步的话画布节点会停在旧编号和旧状态
    syncShotNodeLabels(current.value)
    syncShotNodeStatus(current.value)
    touch()
  }

  function setSubjects(subjects: Subject[]) {
    if (!current.value) return
    current.value.subjects = subjects
    touch()
  }

  function setProps(props: StoryProp[]) {
    if (!current.value) return
    current.value.props = props
    touch()
  }

  function updateProp(id: ID, patch: Partial<StoryProp>) {
    if (!current.value) return
    const list = current.value.props ?? []
    const idx = list.findIndex((item) => item.id === id)
    if (idx === -1) return
    const next = [...list]
    next[idx] = { ...next[idx], ...patch }
    current.value.props = next
    touch()
  }

  function updateSubject(id: ID, patch: Partial<Subject>) {
    if (!current.value) return
    const list = current.value.subjects ?? []
    const idx = list.findIndex((s) => s.id === id)
    if (idx === -1) return
    const next = [...list]
    next[idx] = { ...next[idx], ...patch }
    current.value.subjects = next
    touch()
  }

  function setScenes(scenes: Scene[]) {
    if (!current.value) return
    current.value.scenes = scenes
    touch()
  }

  function updateScene(id: ID, patch: Partial<Scene>) {
    if (!current.value) return
    const list = current.value.scenes ?? []
    const idx = list.findIndex((s) => s.id === id)
    if (idx === -1) return
    const next = [...list]
    next[idx] = { ...next[idx], ...patch }
    current.value.scenes = next
    touch()
  }

  /** 更新单个分镜 */
  function updateShot(id: ID, patch: Partial<Shot>) {
    if (!current.value) return
    const idx = current.value.shots.findIndex((s) => s.id === id)
    if (idx === -1) return
    current.value.shots[idx] = { ...current.value.shots[idx], ...patch }
    if (current.value.canvas && [
      'subjectIds', 'subjectBindingMode', 'sceneId', 'propIds', 'title', 'visualPrompt', 'videoPrompt',
      'voiceover', 'blocking', 'propFocus', 'purpose', 'audioDescription'
    ].some(key => key in patch)) {
      current.value.canvas.edges = projectShotBindingEdges(current.value, current.value.canvas)
    }
    // 标题或顺序变化时同步节点标签，避免参考图名、导出文件名和智能体匹配停留在旧值
    if (patch.title !== undefined || patch.index !== undefined) syncShotNodeLabels(current.value)
    // 资产变化时同步节点状态，避免「选了图/出了片，画布还显示待生成」
    if (
      ['imageAssetId', 'boardAssetId', 'videoAssetId', 'audioAssetId', 'firstFrameAssetId', 'lastFrameAssetId']
        .some((key) => key in patch)
    ) syncShotNodeStatus(current.value)
    touch()
  }

  async function saveShotBindings(id: ID, patch: {
    subjectIds?: ID[]
    subjectBindingMode?: Shot['subjectBindingMode']
    sceneId?: ID | null
    propIds?: ID[] | null
  }) {
    const initial = current.value
    const shot = initial?.shots.find(item => item.id === id)
    if (!initial || !shot) throw new Error('找不到指定镜头')
    if (window.desktop?.isLocalWeb) {
      if (dirty.value) await save()
      if (current.value !== initial || dirty.value) throw new Error('项目状态已变化，请重试')
      await callWorkspace('studio_update_shot_inputs', {
        projectId: initial.id, expectedUpdatedAt: initial.updatedAt, shotId: id, ...patch
      })
      await load(initial.id, () => current.value?.id === initial.id && !dirty.value)
      return
    }
    const { propIds, ...rest } = patch
    updateShot(id, { ...rest, ...(propIds !== undefined ? { propIds: propIds ?? undefined } : {}) })
    try {
      await save()
    } catch (error) {
      if (current.value === initial) updateShot(id, {
        subjectIds: shot.subjectIds, subjectBindingMode: shot.subjectBindingMode,
        sceneId: shot.sceneId, propIds: shot.propIds
      })
      throw error
    }
  }

  async function changeShotBinding(shotId: ID, kind: 'subject' | 'scene' | 'prop', id: ID, connected: boolean) {
    const project = current.value
    const shot = project?.shots.find(item => item.id === shotId)
    if (!project || !shot) throw new Error('找不到指定镜头')
    if (kind === 'scene') {
      await saveShotBindings(shotId, { sceneId: connected ? id : null })
      return
    }
    const ids = new Set(kind === 'subject'
      ? boundSubjectIds(shot, project.subjects ?? [])
      : propsForShot(shot, project).map(item => item.id))
    if (connected) ids.add(id)
    else ids.delete(id)
    if (ids.size > 24) throw new Error('每镜最多绑定 24 张同类卡片')
    await saveShotBindings(shotId, kind === 'subject'
      ? { subjectBindingMode: 'explicit', subjectIds: [...ids] }
      : { propIds: [...ids] })
  }

  /** 改提示词只停掉还没回来的旧任务。已经生成的视频留在节点上，下次点生成才按新词重做。 */
  function setShotVideoPrompt(id: ID, value: string) {
    if (!current.value) return
    const idx = current.value.shots.findIndex((s) => s.id === id)
    if (idx === -1) return
    const shot = current.value.shots[idx]
    const source = shot.videoPrompt ?? ''
    const changed = (shot.compiledVideoPrompt ?? '') !== value
    if (!changed && shot.compiledFromVideoPrompt === source) return
    current.value.shots[idx] = {
      ...shot,
      compiledVideoPrompt: value,
      // 记下这版终稿基于哪版表演稿，之后表演稿再改就能识别出终稿过期
      compiledFromVideoPrompt: source,
      ...(changed ? { videoTaskId: undefined } : {})
    }
    touch()
  }

  function assignShotBoard(shotId: ID, assetId: ID) {
    if (!current.value) return
    const shot = current.value.shots.find((s) => s.id === shotId)
    if (!shot) return
    updateShot(shotId, {
      boardAssetId: assetId,
      boardHistoryIds: pushHistory(shot.boardAssetId, assetId, shot.boardHistoryIds)
    })
  }

  /** 把图片绑到分镜：旧图进历史，不从资源库删除 */
  function assignShotImage(shotId: ID, assetId: ID) {
    if (!current.value) return
    const shot = current.value.shots.find((s) => s.id === shotId)
    if (!shot) return
    updateShot(shotId, {
      imageAssetId: assetId,
      imageHistoryIds: pushHistory(shot.imageAssetId, assetId, shot.imageHistoryIds)
    })
  }

  function assignGeneratedFirstFrame(shotId: ID, assetId: ID, previous: Pick<Shot, 'firstFrameAssetId' | 'imageAssetId'>) {
    const shot = current.value?.shots.find(item => item.id === shotId)
    if (!shot || shot.firstFrameAssetId !== previous.firstFrameAssetId) return false
    const syncMain = shot.imageAssetId === previous.imageAssetId &&
      (!shot.imageAssetId || shot.imageAssetId === shot.firstFrameAssetId)
    updateShot(shotId, {
      firstFrameAssetId: assetId,
      ...(syncMain ? { imageAssetId: assetId, imageHistoryIds: pushHistory(shot.imageAssetId, assetId, shot.imageHistoryIds) } : {})
    })
    return true
  }

  /** 把视频绑到分镜：旧视频进历史，不从资源库删除 */
  function assignShotVideo(shotId: ID, assetId: ID) {
    if (!current.value) return
    const shot = current.value.shots.find((s) => s.id === shotId)
    if (!shot) return
    updateShot(shotId, {
      videoAssetId: assetId,
      videoHistoryIds: pushHistory(shot.videoAssetId, assetId, shot.videoHistoryIds)
    })
  }

  /** 把口播音频绑到分镜 */
  function assignShotAudio(shotId: ID, assetId: ID) {
    if (!current.value) return
    const shot = current.value.shots.find((s) => s.id === shotId)
    if (!shot) return
    updateShot(shotId, {
      audioMode: shot.audioMode ?? 'tts',
      audioAssetId: assetId,
      audioHistoryIds: pushHistory(shot.audioAssetId, assetId, shot.audioHistoryIds)
    })
  }

  /** 设置时间线 */
  function setTimeline(timeline: TimelineClip[]) {
    if (!current.value) return
    current.value.timeline = timeline
    touch()
  }

  function updateClip(id: ID, patch: Partial<TimelineClip>) {
    if (!current.value) return
    const idx = current.value.timeline.findIndex((c) => c.id === id)
    if (idx === -1) return
    current.value.timeline[idx] = { ...current.value.timeline[idx], ...patch }
    touch()
  }

  function addClip(clip: TimelineClip) {
    if (!current.value) return
    current.value.timeline.push(clip)
    touch()
  }

  function removeClip(id: ID) {
    if (!current.value) return
    current.value.timeline = current.value.timeline.filter((c) => c.id !== id)
    touch()
  }

  function setSettings(patch: Partial<ProjectSettings>) {
    if (!current.value) return
    current.value.settings = { ...current.value.settings, ...patch }
    touch()
  }

  function setCaptionSrt(srt: string) {
    if (!current.value) return
    current.value.captionSrt = srt
    touch()
  }

  function setBgm(assetId: string | undefined, prompt?: string) {
    if (!current.value) return
    current.value.bgmAssetId = assetId
    if (!assetId) current.value.bgmClip = undefined
    if (prompt !== undefined) current.value.bgmPrompt = prompt
    touch()
  }

  function setFinalVideo(assetId: string | undefined) {
    if (!current.value) return
    current.value.finalVideoAssetId = assetId
    touch()
  }

  /** 人声方案属于视频生成条件；切换时保留旧视频到历史并要求局部重做。 */
  /** 修改口播配置不删除现有视频；新设置在用户主动生成时生效。 */
  function setShotAudioMode(id: ID, audioMode: ShotAudioMode) {
    if (!current.value) return
    const idx = current.value.shots.findIndex((s) => s.id === id)
    if (idx < 0) return
    const shot = current.value.shots[idx]
    if (shot.audioMode === audioMode) return
    const keepTts = audioMode === 'tts'
    current.value.shots[idx] = {
      ...shot,
      audioMode,
      audioHistoryIds: keepTts
        ? shot.audioHistoryIds
        : pushHistory(shot.audioAssetId, '', shot.audioHistoryIds).filter(Boolean),
      audioAssetId: keepTts ? shot.audioAssetId : undefined
    }
    touch()
  }

  /** 局部修改时只失效受影响的下游产物；旧素材仍留在历史和资源库中。 */
  function invalidateShotAssets(shotId: ID, from: 'image' | 'video' | 'audio') {
    if (!current.value) return
    const idx = current.value.shots.findIndex((s) => s.id === shotId)
    if (idx < 0) return
    const shot = current.value.shots[idx]
    const patch: Partial<Shot> = {}
    if (from === 'image') {
      patch.imageAssetId = undefined
      // 版式描述的是"这张图"，图换了就不该留着旧的格数，否则会被当成四宫格
      patch.imagePromptLayout = undefined
      patch.videoAssetId = undefined
      patch.videoTaskId = undefined
    } else if (from === 'video') {
      patch.videoAssetId = undefined
      patch.videoTaskId = undefined
    } else {
      patch.audioAssetId = undefined
    }
    current.value.shots[idx] = { ...shot, ...patch }
    touch()
  }

  function setAgentState(state: AgentWorkspaceState) {
    if (!current.value) return
    current.value.agentState = state
    touch()
  }

  function patchAgentState(patch: Partial<AgentWorkspaceState>) {
    if (!current.value) return
    const currentState: AgentWorkspaceState = current.value.agentState ?? {
      autonomy: 'gated',
      messages: [],
      tasks: [],
      callBudget: 24
    }
    current.value.agentState = { ...currentState, ...patch }
    touch()
  }

  function patchCanvasNode(
    id: string,
    data: CanvasNodeData
  ) {
    const node = current.value?.canvas?.nodes.find((n) => n.id === id)
    if (!node) return
    const entries = Object.entries(data).filter(([key]) => acceptsCanvasData(node.type, key as keyof CanvasNodeData))
    if (!entries.length) return
    node.data = { ...node.data, ...Object.fromEntries(entries) }
    touch()
  }

  function setBgmClip(clip: Project['bgmClip']) {
    if (!current.value) return
    current.value.bgmClip = clip
    current.value.bgmAssetId = clip?.assetId
    touch()
  }

  function setTextOverlays(items: NonNullable<Project['textOverlays']>) {
    if (!current.value) return
    current.value.textOverlays = items
    touch()
  }

  /** 自由生成节点的 prompt 写入口。视频节点改词后保留已有成片；其它节点仍清掉旧输出。 */
  function setCanvasNodePrompt(id: string, prompt: string) {
    const node = current.value?.canvas?.nodes.find((n) => n.id === id)
    if (!node || !acceptsCanvasData(node.type, 'prompt') || (node.data.prompt ?? '') === prompt) return
    const keepVideo = node.type === 'videoGen'
    node.data = {
      ...node.data,
      prompt,
      ...(keepVideo ? {} : { outputAssetId: undefined }),
      videoTaskId: undefined
    }
    if (!keepVideo) node.status = 'idle'
    node.error = undefined
    touch()
  }

  function removeCanvasNodeRef(nodeId: string, assetId: ID) {
    const p = current.value
    if (!p) return
    removeCanvasNodeReference(p, nodeId, assetId)
    touch()
  }

  /** 把已有素材绑到画布节点成片，跳过生成。公共库素材请先 adopt 进项目。 */
  function assignCanvasNodeAsset(nodeId: string, assetId: ID) {
    const p = current.value
    const node = p?.canvas?.nodes.find((n) => n.id === nodeId)
    if (!p || !node) return
    const shot = node.data.shotId ? p.shots.find((s) => s.id === node.data.shotId) : undefined
    const subject = node.data.subjectId
      ? (p.subjects ?? []).find((s) => s.id === node.data.subjectId)
      : undefined
    const scene = node.data.sceneId ? (p.scenes ?? []).find((s) => s.id === node.data.sceneId) : undefined
    if (node.type === 'imageGen' || node.type === 'videoGen' || node.type === 'audioGen' || node.type === 'compose') {
      node.data = { ...node.data, outputAssetId: assetId }
    } else if (node.type === 'shotImage' && shot) {
      assignShotImage(shot.id, assetId)
    } else if (node.type === 'shotBoard' && shot) {
      assignShotBoard(shot.id, assetId)
    } else if (node.type === 'shotVideo' && shot) {
      assignShotVideo(shot.id, assetId)
    } else if (node.type === 'sceneProps' && scene) {
      updateScene(scene.id, {
        propSheetAssetId: assetId,
        propSheetHistoryIds: pushHistory(
          scene.propSheetAssetId,
          assetId,
          scene.propSheetHistoryIds
        )
      })
    } else if (node.type === 'subject' && subject) {
      updateSubject(subject.id, {
        designSheetAssetId: assetId,
        designSheetHistoryIds: pushHistory(
          subject.designSheetAssetId,
          assetId,
          subject.designSheetHistoryIds
        )
      })
    } else if (node.type === 'scene' && scene) {
      updateScene(scene.id, {
        plateAssetId: assetId,
        plateHistoryIds: pushHistory(scene.plateAssetId, assetId, scene.plateHistoryIds)
      })
    } else {
      return
    }
    node.status = 'success'
    node.error = undefined
    touch()
  }

  function addCanvasNode(node: CanvasNode) {
    if (!current.value) return
    current.value.canvas.nodes = [...current.value.canvas.nodes, node]
    touch()
  }

  function duplicateCanvasNode(id: string): string | null {
    const p = current.value
    const canvas = p?.canvas
    if (!p || !canvas) return null
    const idx = canvas.nodes.findIndex((n) => n.id === id)
    if (idx < 0) return null
    const src = canvas.nodes[idx]
    if (src.type !== 'imageGen' && src.type !== 'videoGen' && src.type !== 'audioGen') return null
    const type = src.type
    const count = canvas.nodes.filter((n) => n.type === type).length + 1
    const common = {
      id: `${type}_${Date.now().toString(36)}`,
      position: {
        x: src.position.x + 280,
        y: src.position.y
      },
      data: {
        label: type === 'imageGen' ? `图片 ${count}` : type === 'audioGen' ? `音频 ${count}` : `视频 ${count}`,
        prompt: src.data.prompt || '',
        referenceAssetIds: [...(src.data.referenceAssetIds ?? [])],
        referenceSourceNodeIds: [...(src.data.referenceSourceNodeIds ?? [])],
        duration: src.data.duration ?? 8,
        aspectRatio: src.data.aspectRatio ?? p.settings.aspectRatio
      },
      status: 'idle' as const,
      parentGroupId: src.parentGroupId
    }
    let copy: CanvasNode
    if (type === 'videoGen') {
      copy = { ...common, type, data: { ...common.data,
        videoModel: normalizeVideoGenModel(src.data.videoModel),
        h3GenMode: normalizeH3GenMode(src.data.h3GenMode),
        firstFrameAssetId: src.data.firstFrameAssetId,
        lastFrameAssetId: src.data.lastFrameAssetId,
        videoResolution: src.data.videoResolution,
        videoProfileId: src.data.videoProfileId,
        videoStillMode: src.data.videoStillMode,
        generateAudio: src.data.generateAudio
      } }
    } else if (type === 'audioGen') {
      copy = { ...common, type, data: { ...common.data,
        ttsModel: src.data.ttsModel,
        referenceAudioAssetId: src.data.referenceAudioAssetId,
        clonedVoiceId: src.data.clonedVoiceId,
        clonedVoiceModel: src.data.clonedVoiceModel
      } }
    } else {
      copy = { ...common, type }
    }
    const nodes = [...canvas.nodes]
    nodes.splice(idx + 1, 0, copy)
    canvas.nodes = nodes
    touch()
    return copy.id
  }

  function removeCanvasNode(id: string) {
    if (!current.value?.canvas) return
    current.value.canvas.nodes = current.value.canvas.nodes.filter((n) => n.id !== id)
    current.value.canvas.edges = current.value.canvas.edges.filter(
      (e) => e.source !== id && e.target !== id
    )
    touch()
  }

  function setCanvas(canvas: CanvasGraph) {
    if (!current.value) return
    current.value.canvas = canvas
    touch()
  }

  function syncCanvasFromProject() {
    if (!current.value) return
    current.value.canvas = rebuildCanvasGraph(current.value, true)
    touch()
  }

  function setAspectRatio(ar: AspectRatio) {
    if (!current.value) return
    const { width, height } = aspectToSize(ar)
    current.value.settings.aspectRatio = ar
    current.value.settings.width = width
    current.value.settings.height = height
    if (current.value.script) current.value.script.aspectRatio = ar
    current.value.canvas = rebuildCanvasGraph(current.value, true)
    touch()
  }

  function setDefaultVideoResolution(resolution: VideoResolution) {
    if (!current.value) return
    current.value.settings.defaultVideoResolution = resolution
    touch()
  }

  function setPlayApproved(ok: boolean) {
    if (!current.value) return
    current.value.playApproved = ok
    touch()
  }

  function touch() {
    if (!current.value) return
    current.value.updatedAt = Math.max(Date.now(), current.value.updatedAt + 1)
    dirty.value = true
  }

  /** 自动保存（watch） */
  let stopWatch: (() => void) | null = null
  let saveTimer: ReturnType<typeof setTimeout> | null = null
  function startAutoSave() {
    if (stopWatch) return
    stopWatch = watch(
      current,
      (p) => {
        if (!p || !dirty.value) return
        if (saveTimer) clearTimeout(saveTimer)
        saveTimer = setTimeout(() => {
          save().catch((e) => console.warn('[project] autosave failed', e))
        }, 200)
      },
      { deep: true }
    )
  }

  function stopAutoSave() {
    stopWatch?.()
    stopWatch = null
    if (saveTimer) {
      clearTimeout(saveTimer)
      saveTimer = null
    }
  }

  /** 主动保存（串行，避免并行生成互相覆盖） */
  let saveChain: Promise<void> = Promise.resolve()
  async function save() {
    const run = async () => {
      if (!current.value) return
      const stamp = current.value.updatedAt
      await saveProject(current.value)
      lastSavedAt.value = Date.now()
      if (current.value.updatedAt === stamp) {
        dirty.value = false
      }
      await refreshList()
    }
    const next = saveChain.then(run, run)
    saveChain = next.catch(() => undefined)
    return next
  }

  /** 删除项目 */
  async function remove(id: string) {
    assertProjectUnlocked()
    if (removingProjects.has(id)) throw new Error('正在移除此项目，请稍候')
    removingProjects.add(id)
    try {
      if (current.value?.id === id) {
        await saveChain
        if (dirty.value) await save()
      }
      await deleteProject(id)
      removalVersion += 1
      if (current.value?.id === id) {
        current.value = null
        dirty.value = false
        setActiveProjectDir(null)
      }
      if (localStorage.getItem(STORAGE_KEY_LAST) === id) localStorage.removeItem(STORAGE_KEY_LAST)
      allProjects.value = allProjects.value.filter((project) => project.id !== id)
      const projects = await refreshList()
      if (projects.some((project) => project.id === id)) throw new Error('项目仍出现在工作区列表中，请检查是否存在重复项目目录')
    } finally {
      removingProjects.delete(id)
    }
  }

  return {
    current,
    allProjects,
    dirty,
    lastSavedAt,
    hydrated,
    isLoaded,
    shotCount,
    totalDuration,
    refreshList,
    ensureLoaded,
    load: (id: string) => changeProject(() => load(id)),
    saveShotBindings,
    changeShotBinding,
    syncFromService: (id: string, shouldApply: () => boolean) => changeProject(() => load(id, shouldApply)),
    create,
    createNamed: (name: string) => changeProject(() => createNamed(name)),
    openFromFolder: (dir: string) => changeProject(() => openFromFolder(dir)),
    lockActiveProject,
    rename,
    setScript,
    setChapters,
    setSelectedChapterIds,
    setProductionChapterIds,
    applyPlayTitle,
    setShots,
    setSubjects,
    setProps,
    updateProp,
    updateSubject,
    setScenes,
    updateScene,
    updateShot,
    setShotVideoPrompt,
    setShotAudioMode,
    assignShotBoard,
    assignShotImage,
    assignGeneratedFirstFrame,
    assignShotVideo,
    assignShotAudio,
    invalidateShotAssets,
    setTimeline,
    updateClip,
    addClip,
    removeClip,
    setSettings,
    setCaptionSrt,
    setBgm,
    setBgmClip,
    setTextOverlays,
    setFinalVideo,
    setAgentState,
    patchAgentState,
    setCanvas,
    patchCanvasNode,
    setCanvasNodePrompt,
    removeCanvasNodeRef,
    assignCanvasNodeAsset,
    addCanvasNode,
    duplicateCanvasNode,
    removeCanvasNode,
    syncCanvasFromProject,
    setAspectRatio,
    setDefaultVideoResolution,
    setPlayApproved,
    save,
    remove: (id: string) => changeProject(() => remove(id)),
    startAutoSave,
    stopAutoSave,
    rememberCurrent
  }
})
