import { effectiveShotAudioMode } from '@/utils/shot'
/**
 * 人设 / 场景底板提示词
 *
 * 媒介跟用户原话走：点了什么画风就按什么材料画。
 * 对得上已知画种时补该画种技法；对不上也锁用户原词，不拿水墨或电影感顶替。
 */

import type { Scene, Shot, Subject, VisualStyle } from '@/types'
import {
  isPhotographicLook,
  lookFromTexts,
  mediumLockText,
  type VisualLook
} from '@/utils/visualStyle'

export function characterSheetPrompt(
  subject: Subject,
  style: VisualStyle,
  hasReferencePhotos: boolean,
  look?: VisualLook
): string {
  const resolved = look ?? lookFromTexts(style)
  const medium = resolved.medium
  const kindHint =
    subject.kind === 'animal'
      ? '同一只动物，品种、斑纹、毛发/羽毛完全一致'
      : subject.kind === 'other'
        ? '同一物体/吉祥物，材质、标志、颜色与轮廓完全一致'
        : '同一人物，脸型、年龄、发型、体型、服装与配饰完全一致'

  const refHint = hasReferencePhotos
    ? '附件参考图已展示该角色，必须严格按参考图复制身份特征，禁止另造一张脸或换装。'
    : ''

  const whiteBg =
    '背景必须是无缝纯白（#FFFFFF），四格同一块白底。禁止灰色/米色影棚、渐变、地面线、阴影底板、风景、室内陈设、路人或任何环境。'

  const layout = isPhotographicLook(resolved)
    ? `横向四幅并排画面，${medium}质感。不出现标题、编号或说明文字。
${whiteBg}
从左到右依次是面部特写、全身正面、全身侧面和全身背面。`
    : `横向四幅并排画面，${medium}媒介。不出现标题、编号或说明文字。
${whiteBg}
从左到右正好四格：面部特写、全身正面、全身侧面、全身背面。身份特征锁死。
材料、笔触、质感必须符合「${medium}」。禁止用真人摄影棚或其他画风冒充。`

  return `${layout}
${refHint}${subject.description}

${kindHint}。四格必须是同一角色、同一服装、同一配色。不要拼贴无关人物，不要画道具拆解图。
${whiteBg}
${mediumLockText(resolved)}`
}

const WHITE_BG_HINT = /纯白背景|#FFFFFF|无缝纯白/
const DRAMLO_BG_HINT = /浅色棚拍|中性灰|影棚背景|干净浅色|浅色干净背景/

/** 角色设计图一律纯白底；旧提示词里的浅色棚拍/灰影棚改掉。 */
export function ensureCharacterSheetWhiteBackground(prompt: string): string {
  const lock =
    '背景必须是无缝纯白（#FFFFFF），四格同一块白底。禁止灰色/米色影棚、渐变、地面线、阴影底板、风景、室内陈设、路人或任何环境。'
  let next = prompt
    .replace(/干净浅色棚拍背景（无风景、无路人）/g, lock)
    .replace(/中性灰影棚背景/g, '无缝纯白背景（#FFFFFF）')
    .replace(/浅色棚拍背景/g, '无缝纯白背景（#FFFFFF）')
    .replace(/浅色干净背景/g, '无缝纯白背景（#FFFFFF）')
  if (!WHITE_BG_HINT.test(next) || DRAMLO_BG_HINT.test(next)) {
    next = `${next.trim()}\n${lock}`
  }
  return next.trim()
}

/** 反复入镜的同一件道具：纯白底正面、侧面、背面、细节。 */
export function propTurnaroundPrompt(prop: { name: string; description?: string }, look?: VisualLook): string {
  const medium = look?.medium || '与全片一致的画风'
  const whiteBg =
    '背景必须是无缝纯白（#FFFFFF），四格同一块白底。禁止灰色/米色影棚、渐变、地面线、阴影底板、风景、室内陈设、人手或任何环境。'
  return `横向四幅道具画面，${medium}质感。不出现标题、编号或说明文字。
${whiteBg}
同一件道具，形状、材质、颜色、磨损和标志完全一致。从左到右四格：正面、侧面、背面、局部细节。不要人物，不要场景，不要把别的物件画进来。
外观锁定：${prop.description || prop.name}`
}

/** 这个角色身上有哪些东西：包、证件、平板、鞋、配饰，单独一张资产图 */
export function characterPropSheetPrompt(
  subject: Subject,
  style: VisualStyle,
  look?: VisualLook
): string {
  const resolved = look ?? lookFromTexts(style)
  const medium = resolved.medium
  const inventory =
    subject.props?.trim() ||
    '从角色描述里抽出穿戴和随身物：包、证件、平板、鞋、帽子、配饰、手里常拿的东西。没有明显物件就画服装关键细节（扣、领、鞋、口袋）。'

  return `浅色干净背景，物件分开陈列，${medium}质感，形状、材质与颜色清楚可见。画面没有标题或说明文字。
清单：${inventory}
角色描述（用来对上款式，不要再画全身人）：${subject.description}

只画物件，不要再画完整人物四视图。每件东西单独完整，便于后面分镜当参考。
${mediumLockText(resolved)}`
}

/** 当前场景里真正需要锁定的少量关键道具；不把普通陈设拆成资产。 */
export function scenePropSheetPrompt(
  scene: Scene,
  style: VisualStyle,
  look?: VisualLook
): string {
  const resolved = look ?? lookFromTexts(style)
  const medium = resolved.medium
  const inventory = scene.props?.trim() || '无明确关键道具'

  return `独立陈列的道具画面，${medium}媒介，画面没有标题或说明文字。
只展示本场景中明显出现、被人物拿取或使用、或者推动剧情的关键道具，最多 4 件：${inventory}
场景语境：${scene.description}

像影视美术部门的道具设定板：每件道具单独完整展示，清楚锁定形状、尺寸比例、材质、颜色和磨损痕迹。纯净中性背景，不画人物，不画完整场景，不添加清单之外的杯子、普通家具、装饰、衣服或零碎杂物。若清单只有一件，就只设计这一件，不要自行凑数。
${mediumLockText(resolved)}`
}

/** 单镜故事看板：六格黑白草图，锁定同一镜头内的动作时间线。 */
export function shotBoardPrompt(
  shot: Pick<
    Shot,
    | 'title'
    | 'duration'
    | 'cameraMovement'
    | 'shotSize'
    | 'blocking'
    | 'continuity'
    | 'visualPrompt'
    | 'videoPrompt'
    | 'boardPrompt'
    | 'propFocus'
    | 'voiceover'
    | 'audioMode'
  >,
  _style: VisualStyle,
  _look?: VisualLook,
  options?: { filmChapter?: boolean; aspectRatio?: string }
): string {
  const body = (shot.boardPrompt || shot.visualPrompt || shot.title).trim()
  const size = shot.shotSize ? `景别：${shot.shotSize}。` : ''
  const blocking = shot.blocking ? `站位：${shot.blocking}。` : ''
  const prop = shot.propFocus && shot.propFocus !== '无' ? `关键道具：${shot.propFocus}。` : ''
  const continuity = shot.continuity ? `动作承接：${shot.continuity}。` : ''
  const performance = shot.videoPrompt?.trim()
    ? `动态表演：${shot.videoPrompt.trim()}。`
    : ''
  const voiceMode = effectiveShotAudioMode(shot)
  const dialogue = shot.voiceover?.trim()
    ? voiceMode === 'h3-voiceover' || voiceMode === 'tts' || voiceMode === 'none'
      ? '出镜人物闭嘴，通过停步、回头和视线变化表现听见声音后的反应。'
      : (() => {
          const speaker = shot.voiceover.trim().match(/^([^：:\n]{1,12})[：:]/)?.[1]?.trim()
          return `先构图突出${speaker || '说话者'}的脸和嘴。其他人物通过视线和表情回应。画面没有对白文字或气泡。`
        })()
    : ''
  const camera =
    shot.cameraMovement === 'push'
      ? '六格中人物和关键物逐格自然变大，表现同一次缓慢推进。'
      : shot.cameraMovement === 'pull'
        ? '六格中画面逐格自然变宽，表现同一次缓慢拉远。'
        : shot.cameraMovement === 'pan'
          ? '六格保持人物比例，背景与构图逐格横向移动，表现同一次平摇。'
          : shot.cameraMovement === 'tilt'
            ? '六格保持人物比例，构图逐格纵向移动，表现同一次俯仰。'
            : shot.cameraMovement === 'orbit'
              ? '六格中观察角度逐格小幅变化，保持同一人物、同一空间关系。'
              : '六格机位和景别完全相同，只让人物动作、表情与道具状态连续变化。'

  if (options?.filmChapter) {
    const longAction =
      shot.duration >= 9 || /蒙太奇|快速切|连续数|反复|多次|逐渐/.test(`${body}${performance}`)
    const count = longAction ? 8 : 4
    const layout = longAction ? '4 列 × 2 行' : '2 列 × 2 行'
    const beats = longAction
      ? '八格依次表现：建立环境、人物初态、犹豫或准备、动作开始、核心动作、身体或环境反应、动作收束、清楚的结束姿势。'
      : '四格依次表现：建立与初态、犹豫或准备、核心动作、反应与结束姿势。'
    const filmCamera =
      shot.cameraMovement === 'static'
        ? `${count} 格机位、焦段和构图完全固定，只让人物动作、表情、雨水、蒸汽等环境细节连续变化。`
        : `用 ${count} 个连续时间采样清楚表现同一次「${shot.cameraMovement}」运镜，幅度克制，禁止跳轴或突然换机位。`

    return `彩色画面等分为 ${layout}，共 ${count} 格。由左上到右下呈现同一个连续镜头或同一段蒙太奇。

画面风格：正常上色的现实主义电影画面，真实自然光、生活化人物、符合剧本年代和地区的服装建筑与用品；像导演与摄影指导使用的成片预演，不是黑白线稿、漫画或海报。
版式要求：格子大小整齐，细黑线分隔；不出现标题、编号、时间码、字幕、对白气泡、说明文字、logo、水印或任何可读文字。

${beats}
${filmCamera}

连续性硬约束：所有格必须是同一地点、同一时段、同一批人物、同一张脸、同一年龄、同一发型服装和同一道具。天气、地面干湿、污渍、伤痛、饥饿、衣物湿润、光源方向、屏幕方向、人物站位和场景主锚必须逐格承接，禁止凭空增加人物、换装、换边或改变建筑结构。
${size}${blocking}${prop}${continuity}${performance}${dialogue}
${body}

参考图必须用上：脸、年龄、体型和服装跟角色设计图；建筑、地面、天气、材料和光跟场景锚点图。禁止另造人物或地点。最终只输出这一张彩色 ${count} 宫格电影故事看板。`
  }

  return `白底线稿等分为三列两行，由左上到右下呈现同一镜头的六个连续动作瞬间。

画面风格：白底、黑色或浅灰铅笔线稿，线条简洁，少量灰面辅助空间关系；人物用清楚的简笔轮廓表达动作。禁止精致上色、禁止电影海报、禁止写实渲染。
版式要求：六格大小整齐，细线分隔。画面内不出现标题、编号、时间码、字幕、对白气泡、说明文字、水印或任何可读文字。

六格动作节奏：
第一格建立 0 秒的起始构图和人物初始状态；第二格做动作准备；第三格动作开始；第四格完成核心动作；第五格表现对白/反应和道具状态变化；第六格锁定本镜结束状态，供下一镜或视频末帧承接。
${camera}

连续性硬约束：六格必须是同一地点、同一时段、同一批人物、同一张脸、同一发型服装、同一道具；人物不能凭空增加、消失、换装或换边，道具状态只能沿时间顺序变化。保持屏幕方向、站位关系、光源方向和场景主锚一致。
${size}${blocking}${prop}${continuity}${performance}${dialogue}
${body}

参考图必须用上：人物轮廓、脸、服装跟角色设计图；地点、光、地面跟场景图。禁止另造人物或地点。最终只输出这一张六格黑白故事看板。`
}

/** 普通创作关键帧：2×2 四宫格，正好 4 格，不能多。 */
export function shotKeyframePrompt(
  shot: Pick<
    Shot,
    | 'title'
    | 'duration'
    | 'cameraMovement'
    | 'shotSize'
    | 'blocking'
    | 'continuity'
    | 'visualPrompt'
    | 'videoPrompt'
    | 'imagePrompt'
    | 'propFocus'
    | 'voiceover'
    | 'audioMode'
  >,
  look?: VisualLook,
  _options?: { aspectRatio?: string }
): string {
  const visualOnly = (text: string) => {
    let result = text.replace(/<d>[\s\S]*?<\/d>/gi, '')
    for (const line of (shot.voiceover || '').split('\n').map((item) => item.trim()).filter(Boolean)) {
      result = result.split(line).join('')
      const spoken = line.replace(/^[^：:\n]{1,20}[：:]\s*/, '')
      if (spoken.length > 2) result = result.split(spoken).join('')
    }
    return result.trim()
  }
  const body = visualOnly(shot.visualPrompt || shot.title)
  const size = shot.shotSize ? `景别：${shot.shotSize}。` : ''
  const blocking = shot.blocking ? `站位：${shot.blocking}。` : ''
  const prop = shot.propFocus && shot.propFocus !== '无' ? `关键道具：${shot.propFocus}。` : ''
  const continuity = shot.continuity ? `动作承接：${shot.continuity}。` : ''
  const performance = shot.videoPrompt?.trim() ? `动态表演：${visualOnly(shot.videoPrompt)}。` : ''
  const voiceMode = effectiveShotAudioMode(shot)
  const dialogue = shot.voiceover?.trim()
    ? voiceMode === 'h3-voiceover' || voiceMode === 'tts' || voiceMode === 'none'
      ? '这是画外音镜头。只画人物动作与表情，不绘制对白、字幕或气泡。'
      : '这是人物交流场景。只通过人物动作、视线与表情表现互动，不绘制台词、字幕或气泡；不要把整场交流都安排给同一个人。'
    : ''
  const camera =
    shot.cameraMovement === 'static'
      ? '四格机位、焦段和构图完全固定，只让人物动作、表情和环境细节连续变化。'
      : `用 4 个连续时间采样清楚表现同一次「${shot.cameraMovement}」运镜，幅度克制，禁止跳轴。`
  const medium = look ? mediumLockText(look) : ''

  return `彩色画面等分为两列两行，每格有完整构图和细线边界。

四格属于同一个连续镜头的四个时间点，不是四张无关海报，也不是空白格。
左上呈现人物初态，右上呈现动作准备，左下呈现核心动作，右下呈现反应和结束姿势。
${camera}

版式：格子大小整齐，细线分隔；不出现标题、编号、时间码、字幕、对白气泡、说明文字、logo 或水印。禁止空白格子、纯色空格或未画完的框。

连续性：同一地点、同一时段、同一批人物、同一张脸、同一服装、同一道具。禁止另造人、换装、换景。
${size}${blocking}${prop}${continuity}${performance}${dialogue}
${body}

参考图必须用上：脸、体型、服装跟人设图；地点、地面、光线跟场景图。最终只输出这一张正好 4 格的彩色四宫格关键帧。
${medium}`.trim()
}

export function scenePlatePrompt(scene: Scene, style: VisualStyle, look?: VisualLook): string {
  const resolved = look ?? lookFromTexts(style)
  const medium = resolved.medium
  const lighting = scene.lighting?.trim() || '与文案匹配的主光源'
  const main = scene.mainAnchor?.trim() || '这个地点里最能对焦的实物'
  const extra = scene.anchors?.trim()
  const grid = extra || `左上是环境全景，右上是${main}的特写，左下是地面和通道，右下是${lighting}照亮的建筑或物件。`

  const photo = isPhotographicLook(resolved)
  const lead = photo
    ? `两列两行的四幅环境画面，格间细线，${medium}质感。`
    : `两列两行的四幅环境画面，格间细线，材料、笔触与质感符合${medium}媒介。`

  return `${lead}
${scene.description}
${grid}

四格是同一地点、同一时段、同一套材质。画面没有标题、编号、说明文字、水印或标志。
空场景，画面中没有人形、人影或反射中的人。
只画建筑、家具、地面、门窗、主锚物件和光。一个活物都没有。
禁止把四格合成一张宽画幅风景。
${mediumLockText(resolved)}`
}
