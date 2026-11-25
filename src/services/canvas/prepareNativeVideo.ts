import { callWebVideoQueue, callWorkspace } from '@/services/localWebStorage'
import { useProjectStore, useSettingsStore } from '@/stores'
import type { Shot } from '@/types/project'
import { collectShotVideoEntries } from '@/utils/consistency'
import { shotUsesFrames } from '@/utils/videoStill'

type WorkflowInputs = { hasFirstFrame?: boolean; hasLastFrame?: boolean; referenceCount?: number; maxReferences?: number }

export async function prepareNativeShotVideos(targets: Shot[]): Promise<boolean> {
  if (!window.desktop?.isLocalWeb || !targets.length) return false
  const project = useProjectStore()
  const settings = useSettingsStore()
  const current = project.current
  if (!current || (current.pipelineMode === 'smart-speech' && current.speechWorkflow?.mode === 'digital-human')) return false
  const profileIds = targets.map(shot =>
    shot.videoProfileId ||
    settings.config.activeVideoId)
  const native = profileIds.map(id => settings.config.videos.find(profile => profile.id === id)?.adapter?.type === 'comfyui-prompt')
  if (!native.some(Boolean)) return false
  if (!native.every(Boolean)) throw new Error('此批混用了本机 ComfyUI 和其他供应商，请拆批执行')
  if (project.dirty) await project.save()
  if (project.current !== current || project.dirty) throw new Error('项目已切换或还有未保存的修改')
  const revision = current.updatedAt
  const profiles = new Map<string, WorkflowInputs>()
  for (const id of new Set(profileIds)) {
    if (!id) throw new Error('请选择视频工作流')
    const model = await callWorkspace<{ workflow?: WorkflowInputs; ready: boolean; endpointIssue?: string }>(
      'studio_describe_model', { kind: 'video', profileId: id })
    if (!model.ready || !model.workflow) throw new Error(model.endpointIssue || '所选工作流映射未就绪')
    profiles.set(id, model.workflow)
  }
  if (project.current !== current || project.dirty || current.updatedAt !== revision) {
    throw new Error('项目在预检前已变更，请重试')
  }
  const ordered = [...current.shots].sort((a, b) => a.index - b.index)
  const shots = targets.map((shot, index) => {
    const profileId = profileIds[index]!
    const workflow = profiles.get(profileId)!
    const previous = ordered[ordered.findIndex(item => item.id === shot.id) - 1]
    const takesTail = !workflow.hasLastFrame && shotUsesFrames(shot, current) && current.settings.h3ShotChainMode === 'fl2va'
      && !shot.firstFrameAssetId && previous?.sceneId && previous.sceneId === shot.sceneId
    const hasFrameSlots = workflow.hasFirstFrame || workflow.hasLastFrame
    const maxReferences = workflow.maxReferences ?? (hasFrameSlots ? 0 : workflow.referenceCount ?? 0)
    const mode = workflow.hasLastFrame ? 'first-last-frame' : workflow.hasFirstFrame
      ? takesTail ? 'previous-tail' : 'first-frame' : maxReferences ? 'reference' : 'text'
    const first = shot.firstFrameAssetId || shot.imageAssetId
    const references = maxReferences > 0 ? collectShotVideoEntries(shot, current)
      .filter(item => !hasFrameSlots || (item.id !== first && item.id !== shot.lastFrameAssetId))
      .slice(0, maxReferences)
      .map(item => item.id) : undefined
    return { shotId: shot.id, profileId, mode,
      ...(references ? { referenceAssetIds: references } : {}),
      ...(mode === 'first-frame' && first ? { firstFrameAssetId: first } : {})
    }
  })
  await callWebVideoQueue('studio_prepare_video', {
    projectId: current.id, requestId: crypto.randomUUID(), expectedUpdatedAt: revision, shots
  })
  return true
}
