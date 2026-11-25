import type { CanvasNode, CanvasNodeOf, Project, Shot } from '@/types'
import type { VideoProfile } from '@/stores/settings'
import {
  normalizeH3GenMode,
  normalizeVideoGenModel,
  stillModeForVideoModel,
  type VideoStillMode
} from '@/types/canvas'

export function stillModeFromNode(node?: Pick<CanvasNode, 'data'> | null): VideoStillMode {
  const raw = node?.data.videoStillMode
  if (raw === 'identity' || raw === 'keyframe' || raw === 'storyboard') return raw
  return stillModeForVideoModel(normalizeVideoGenModel(node?.data.videoModel))
}

export function shotVideoNode(project: Project | null | undefined, shotId: string) {
  return project?.canvas?.nodes.find((n): n is CanvasNodeOf<'shotVideo'> =>
    n.type === 'shotVideo' && n.data.shotId === shotId)
}

/** For native ComfyUI, match the connected inputs used by the server preflight. */
export function videoFrameInputMode(profile: VideoProfile | undefined): 'first' | 'pair' | 'reference' | 'unknown' {
  if (!profile) return 'unknown'
  if (profile.adapter?.type !== 'comfyui-prompt') return profile.options?.lastFrameRequired ? 'pair' : 'unknown'
  try {
    const raw = JSON.parse(profile.adapter.workflowTemplate || '')
    const nodes = Object.values(raw.prompt ?? raw) as Array<{ class_type: string; inputs: Record<string, unknown> }>
    if (!nodes.length || !nodes.every(node => typeof node?.class_type === 'string' && node.inputs && typeof node.inputs === 'object')) return 'unknown'
    const first = nodes.some(({ class_type, inputs }) => Array.isArray(inputs.first_frame) ||
      (class_type.startsWith('MiniMaxH3') && inputs.task_type === 'I2VA' && Array.isArray(inputs['ref_images.ref_image_0'])))
    const last = nodes.some(({ inputs }) => Array.isArray(inputs.last_frame) || Array.isArray(inputs.end_frame))
    return first ? (last ? 'pair' : 'first') : (last ? 'unknown' : 'reference')
  } catch {
    return 'unknown'
  }
}

export function stillModeForShot(shot: Shot, project: Project): VideoStillMode {
  return stillModeFromNode(shotVideoNode(project, shot.id))
}

export function shotMissingStill(shot: Shot, project: Project): boolean {
  // 走首尾帧的镜头视频直接吃起始帧/结束帧，不要求关键帧或看板
  if (shotUsesFrames(shot, project)) return false
  const mode = stillModeForShot(shot, project)
  if (mode === 'storyboard') return !shot.boardAssetId
  if (mode === 'keyframe') return !shot.imageAssetId
  return false
}

export function stillTypeForMode(mode: VideoStillMode): 'shotBoard' | 'shotImage' | null {
  if (mode === 'storyboard') return 'shotBoard'
  if (mode === 'keyframe') return 'shotImage'
  return null
}

export function shotsMissingKeyframe(project: Project): Shot[] {
  return (project.shots ?? []).filter(
    (shot) =>
      !shotUsesFrames(shot, project) && stillModeForShot(shot, project) === 'keyframe' && !shot.imageAssetId
  )
}

export function shotsMissingStoryboard(project: Project): Shot[] {
  return (project.shots ?? []).filter(
    (shot) =>
      !shotUsesFrames(shot, project) && stillModeForShot(shot, project) === 'storyboard' && !shot.boardAssetId
  )
}

/**
 * 这一镜是否走首尾帧出片：镜头自己设过就按镜头的来；
 * 没设过时才跟随项目默认。项目级首尾帧链只是默认值，不再强制全部镜头。
 */
export function shotUsesFrames(shot: Shot, project: Project | null | undefined): boolean {
  if (!project) return false
  if (shot.h3GenMode) return normalizeH3GenMode(shot.h3GenMode) === 'fl2va'
  return (project.settings?.h3ShotChainMode || 'auto') === 'fl2va'
}

/**
 * 这个项目要不要「首尾帧」这一组：只要有一镜按镜头设置走首尾帧就要。
 * 一处定义，画布分组、流水线下一步、智能体判断都用它，免得三边判断不一致。
 */
export function needsShotFrames(project: Project | null | undefined): boolean {
  if (!project) return false
  return (project.shots ?? []).some((shot) => shotUsesFrames(shot, project))
}

/** 还缺起始帧或结束帧的镜头（只算走首尾帧的镜头，混合模式下普通镜头不算缺） */
export function shotsMissingFrames(project: Project | null | undefined): Shot[] {
  if (!project) return []
  return (project.shots ?? []).filter(
    (shot) => shotUsesFrames(shot, project) && (!shot.firstFrameAssetId || !shot.lastFrameAssetId)
  )
}

/** 首尾帧这一站是否还挂着活：要首尾帧，且还有镜头没画完 */
export function frameStagePending(project: Project | null | undefined): boolean {
  return needsShotFrames(project) && shotsMissingFrames(project).length > 0
}
