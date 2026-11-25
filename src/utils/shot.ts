import type { Shot, ShotAudioMode } from '@/types'
import { composeShotVisualPrompt } from '@/utils/shotPrompt'

export {
  composeVideoPerformance,
  synthesizeVideoPerformance,
  isVideoPerformanceThin,
  VIDEO_PERFORMANCE_SPEC,
  assembleShotVideoPrompt,
  videoPerformanceSpec,
  videoPromptJsonHint,
  editableVideoPrompt
} from '@/utils/shotPrompt'
export type { VideoPromptStyle } from '@/utils/shotPrompt'

/**
 * 终稿是否基于旧版表演稿编译。
 * 历史数据没有 compiledFromVideoPrompt，视为未记录、不判断，避免整片误报。
 */
export function compiledPromptStale(
  shot: Pick<Shot, 'compiledVideoPrompt' | 'compiledFromVideoPrompt' | 'videoPrompt'>
): boolean {
  if (!(shot.compiledVideoPrompt || '').trim()) return false
  if (shot.compiledFromVideoPrompt === undefined) return false
  return (shot.videoPrompt || '').trim() !== shot.compiledFromVideoPrompt.trim()
}

/**
 * 镜头在画布节点、属性面板、口播节点上显示的标题。
 * 标题自带镜号（如「01｜0:00-0:03｜…」）时不再加序号，避免出现「1. 01｜0:00-0:03｜…」。
 */
export function shotDisplayTitle(shot: Pick<Shot, 'title' | 'index'>): string {
  const title = (shot.title || '').trim()
  if (!title) return `第 ${shot.index + 1} 镜`
  return /^\d+\s*[｜|]/.test(title) ? title : `${shot.index + 1}. ${title}`
}

/** 配图实际使用的提示词：独立覆盖优先，否则用分镜画面描述 */
export function effectiveImagePrompt(shot: Shot): string {
  return composeShotVisualPrompt(shot, 'image')
}

/** 视频内容表现：谁做什么、说什么、镜头怎么走。绝不回退成配图静帧词 */
export function effectiveVideoPrompt(shot: Shot): string {
  return composeShotVisualPrompt(shot, 'video')
}

export function isDigitalHumanSpeechProject(
  project?: { pipelineMode?: string; speechWorkflow?: { mode?: string } | null } | null
): boolean {
  return project?.pipelineMode === 'smart-speech' && project.speechWorkflow?.mode === 'digital-human'
}

/** 没有指定人声模式时，有台词的镜头使用出镜对白。 */
export function effectiveShotAudioMode(shot: Pick<Shot, 'audioMode' | 'voiceover'>): ShotAudioMode {
  return shot.audioMode ?? (shot.voiceover?.trim() ? 'h3-dialogue' : 'none')
}

export function h3VoiceMode(shot: Shot): 'dialogue' | 'voiceover' | 'none' {
  const mode = effectiveShotAudioMode(shot)
  if (mode === 'h3-dialogue') return 'dialogue'
  if (mode === 'h3-voiceover') return 'voiceover'
  return 'none'
}

/** 说话者不在画面里时，即使标成对白也按画外音处理，避免把被叫的人当成开口者。 */
export function resolveH3VoiceMode(shot: Shot, onScreenNames: string[] = []): 'dialogue' | 'voiceover' | 'none' {
  const mode = h3VoiceMode(shot)
  if (mode !== 'dialogue') return mode
  const speaker = (shot.voiceover || '').match(/^([^：:\n]{1,12})[：:]/)?.[1]?.trim()
  if (!speaker) return mode
  const onScreen = onScreenNames.some((name) => name === speaker || speaker.includes(name) || name.includes(speaker))
  return onScreen ? 'dialogue' : 'voiceover'
}

export function needsExternalTts(
  shot: Shot,
  project?: { pipelineMode?: string; speechWorkflow?: { mode?: string } | null } | null
): boolean {
  if (isDigitalHumanSpeechProject(project)) return false
  return effectiveShotAudioMode(shot) === 'tts' && Boolean(shot.voiceover?.trim())
}

/** 把旧素材推进历史，当前换成新的（去重、最多留 12 条） */
export function pushHistory(
  currentId: string | undefined,
  nextId: string,
  history: string[] | undefined
): string[] {
  const list = [...(history ?? [])]
  if (currentId && currentId !== nextId && !list.includes(currentId)) {
    list.unshift(currentId)
  }
  return list.filter((id) => id !== nextId).slice(0, 12)
}
