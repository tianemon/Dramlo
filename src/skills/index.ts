/**
 * 画布智能体 Skill：按用户意图只加载相关 playbook。
 * 正文是我们自己写的现场手册，不是外站提示词。
 */

import characterRaw from './codex-short-drama/character-consistency.md?raw'
import pipelineRaw from './codex-short-drama/SKILL.md?raw'
import qaRaw from './codex-short-drama/qa-verify.md?raw'
import shotImageRaw from './codex-short-drama/shot-image.md?raw'
import storyboardPanelRaw from './codex-short-drama/storyboard-panel.md?raw'
import shotVideoRaw from './codex-short-drama/shot-video.md?raw'
import storyboardRaw from './codex-short-drama/storyboard-director.md?raw'
import screenwriterRaw from './codex-short-drama/ai-short-film-screenwriter.md?raw'
import h3PromptRaw from './codex-short-drama/h3-prompt-writing.md?raw'

export interface Skill {
  id: string
  name: string
  description: string
  triggers: string[]
  body: string
}

export interface SkillSelectContext {
  userText: string
  pipelineMode?: string
  workflowMode?: string
  hasShots: boolean
  missingSheets: boolean
  missingPlates: boolean
  missingImages: boolean
  missingVideos: boolean
}

interface Frontmatter {
  name: string
  description: string
  triggers: string[]
}

function parseFrontmatter(raw: string): { meta: Frontmatter; body: string } {
  const match = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/)
  if (!match) {
    return {
      meta: { name: 'unknown', description: '', triggers: [] },
      body: raw.trim()
    }
  }
  const yaml = match[1]
  const pick = (key: string) => {
    const line = yaml.split(/\r?\n/).find((l) => l.startsWith(`${key}:`))
    return line ? line.slice(key.length + 1).trim() : ''
  }
  const triggers = pick('triggers')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
  return {
    meta: {
      name: pick('name') || 'unknown',
      description: pick('description'),
      triggers
    },
    body: match[2].trim()
  }
}

function toSkill(raw: string): Skill {
  const { meta, body } = parseFrontmatter(raw)
  return {
    id: meta.name,
    name: meta.name,
    description: meta.description,
    triggers: meta.triggers,
    body
  }
}

export const ALL_SKILLS: Skill[] = [
  toSkill(pipelineRaw),
  toSkill(screenwriterRaw),
  toSkill(storyboardRaw),
  toSkill(characterRaw),
  toSkill(storyboardPanelRaw),
  toSkill(shotImageRaw),
  toSkill(shotVideoRaw),
  toSkill(h3PromptRaw),
  toSkill(qaRaw)
]

const BY_ID = new Map(ALL_SKILLS.map((s) => [s.id, s]))

export function getSkill(id: string): Skill | undefined {
  return BY_ID.get(id)
}

export function getSkillBody(id: string): string {
  return BY_ID.get(id)?.body ?? ''
}

function triggeredByText(skill: Skill, text: string): boolean {
  return skill.triggers.some((t) => t && text.includes(t.toLowerCase()))
}

/**
 * 按意图挑选要塞进上下文的 Skill。
 * 目录始终可见；正文只加载本轮匹配到的手册。
 */
export function selectSkills(ctx: SkillSelectContext): Skill[] {
  const text = ctx.userText.trim().toLowerCase()
  const filmChapter = ctx.pipelineMode !== 'remake' && ctx.workflowMode === 'film-chapter'
  const picked = new Set<string>()
  for (const skill of ALL_SKILLS) {
    if (triggeredByText(skill, text)) {
      picked.add(skill.id)
    }
  }

  // 同一请求可以跨工种。只按目标补选，缺图状态用于全流程接手。
  if (/^剧本$|(?:写|修订|润色|诊断).{0,5}剧本|(?:改|修|润色).{0,8}(?:对白|台词)/.test(text)) picked.add('ai-short-film-screenwriter')
  if (/^人设$|出人设|角色设定图/.test(text)) picked.add('character-consistency')
  if (/^分镜$|(?:拆|排|整理).{0,5}分镜|(?:调整|修改|改).{0,6}(?:运镜|景别|切点)/.test(text)) picked.add('storyboard-director')
  if (!filmChapter && text === '线稿') picked.add('storyboard-panel')
  if (/^视频$|第.{1,6}镜.{0,5}(?:视频|动起来)|(?:生成|制作).{0,5}视频/.test(text)) picked.add('shot-video')
  if (/(?:画出|画一下|画)第.{1,6}镜|第.{1,6}镜画出来/.test(text)) picked.add('shot-image')
  if (/^(?:都生成|全部生成|全做|缺的都补上)[。！! ]*$/.test(text)) picked.add('pipeline-batch')
  if (picked.has('pipeline-batch')) {
    picked.add('qa-verify')
    if (!ctx.hasShots) {
      picked.add('ai-short-film-screenwriter')
      picked.add('storyboard-director')
    } else if (ctx.missingSheets || ctx.missingPlates) {
      picked.add('character-consistency')
    } else if (ctx.missingImages) {
      picked.add('shot-image')
      if (!filmChapter) picked.add('storyboard-panel')
    } else if (ctx.missingVideos) {
      picked.add('shot-video')
    }
  }
  if (filmChapter && ([...picked].some((id) => id !== 'qa-verify') || /电影章节|彩色故事板|八宫格|线稿/.test(text))) {
    picked.add('storyboard-panel')
  }
  return ALL_SKILLS.filter((skill) => picked.has(skill.id))
}

export function catalogPrompt(): string {
  return ALL_SKILLS.map((s) => `- ${s.id}：${s.description}`).join('\n')
}

export function bodiesPrompt(skills: Skill[]): string {
  if (!skills.length) return ''
  return skills.map((s) => `### Skill: ${s.id}\n${s.body}`).join('\n\n')
}

export function skillContextForAgent(ctx: SkillSelectContext): {
  selected: Skill[]
  prompt: string
} {
  const selected = selectSkills(ctx)
  const prompt = `【可调用 Skill 目录】
${catalogPrompt()}

【本轮已加载】${selected.map((s) => s.id).join('、')}
按已加载手册做事，不要发明目录里没有的流程。

${bodiesPrompt(selected)}`
  return { selected, prompt }
}
