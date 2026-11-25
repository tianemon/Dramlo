import { z } from 'zod'

const id = z.string().trim().min(1).max(160)
const kind = z.enum(['llm', 'vision', 'image', 'video', 'tts', 'music'])
const page = { projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), offset: z.number().int().min(0).default(0).describe('分页起点，从 0 开始。继续读取时使用上一次返回的 nextOffset。'), limit: z.number().int().min(1).max(50).default(20).describe('本页最大条目数。') }
const optionalPage = { projectId:page.projectId, offset:z.number().int().min(0).optional().describe('分页起点，默认 0。继续读取时使用 nextOffset。'), limit:z.number().int().min(1).max(50).optional().describe('本页上限，默认 20。') }
const endpoint = z.url().max(2048).refine((value) => {
  const url = new URL(value)
  return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash
}, '请输入不含凭据、查询参数和片段的 HTTP(S) 地址')

const exactRevision = z.number().int().nonnegative().describe('传已核对的 updatedAt 原值。修订冲突时重读项目。')
const autoRevision = z.union([exactRevision, z.literal('auto')])
  .describe('优先传已核对的项目修订号。auto 表示使用服务端本次读取的版本。按镜号写入时会使用当前顺序。')
const revision = { projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), expectedUpdatedAt: autoRevision }
const approvalRevision = { ...revision, expectedUpdatedAt: exactRevision }
const previewRevision = { ...revision, expectedUpdatedAt: autoRevision.describe('预检可传 auto。确认时传已核对的数字项目修订号。') }
const bindingReview = z.string().trim().min(1).max(500).optional()
  .describe('生成前人物对账的人工核对原因。明确是画外提及、替用参考或有意不用人物参考时才填写。预检仍会回显提示。')
const shotDraft = z.strictObject({
  id: z.uuid().describe('新分镜的 UUID。每镜生成一个，重试前先用该 ID 查询是否已写入。'),
  title: z.string().trim().min(1).max(160).describe('分镜的简短标题，例如「父亲深夜推门」。概括该镜动作。'),
  visualPrompt: z.string().trim().min(1).max(12000).describe('本镜实际可见的画面。写角色站位与朝向、动作、景别、场景锚点、光线及初态到末态的变化。'),
  videoPrompt: z.string().trim().min(1).max(12000).optional().describe('镜头视频提示词草稿。原生视频预检读取 compiledVideoPrompt。'),
  compiledVideoPrompt: z.string().trim().min(1).max(12000).optional().describe('已审定、直接提交给视频工作流的提示词。尚未定稿时省略，之后用 studio_set_video_prompt 保存。'),
  voiceover: z.string().max(12000).default('').describe('本镜实际说出的对白或旁白，保留原文。不说话时填空字符串。'),
  audioDescription: z.string().max(4000).default('').describe('本镜的环境音和动作音，例如雨声、木船轻晃。没有则留空。'),
  duration: z.number().positive().max(120).describe('持续时长，单位为秒。'),
  cameraMovement: z.enum(['static', 'push', 'pull', 'pan', 'tilt', 'orbit']).describe('本镜运镜。static 固定，push 推近，pull 拉远，pan 摇移，tilt 俯仰，orbit 环绕。'),
  subjectIds: z.array(id).max(24).default([]).describe('镜头里实际出场的已有角色卡 ID。没有出场人物时可传空数组。'),
  propIds: z.array(id).max(24).optional().describe('本镜实际出场的道具卡 ID，由 studio_project_items 的 props 查询。')
})
const projectAsset = { id: z.uuid().describe('新建资产使用 UUID；更新现有资产使用该资产已保存的 UUID。'), name: id.describe('用户在界面看到的名称。填写有辨识度的实际名称。'), description: z.string().trim().min(1).max(4000).describe('可复用的资产描述，填写可见外观、材质、位置或时间氛围等具体特征。') }
const videoJob = { projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), requestId: z.uuid().describe('本次任务或批次的去重 ID。新批次生成 UUID；确认、暂停、查询和恢复使用同一 ID。') }
const imageTarget = {
  targetType: z.enum(['scene','shot-image','subject','prop','shot-board','shot-first','shot-last']).describe('目标槽位：场景锚点、镜头关键帧、人物图、道具图、看板、首帧或尾帧'),
  targetId: id.describe('对应的场景、镜头、人物或道具 ID')
}
const generationImageTarget = {
  targetType: z.enum(['scene','shot-image','subject','prop','shot-board','shot-first','shot-last','canvas-node'])
    .describe('生图目标槽位。canvas-node 指自由图片或合成节点，结果归到对应节点。'),
  targetId: id.describe('素材卡或镜头 ID，镜头也支持 #05。canvas-node 填画布节点 ID。')
}
const nodePosition = z.strictObject({
  x: z.number().describe('画布横坐标。'), y: z.number().describe('画布纵坐标。')
}).optional().describe('画布绝对坐标；省略时自动定位。')
const captureApprovalMode = z.enum(['batch','scene']).optional().describe('batch 预览并确认整批一次（默认）；scene 逐场预览和确认。')
const captureSceneIndex = z.number().int().nonnegative().optional().describe('场次索引，从 0 开始。逐场准备或标记资产阶段时使用。')
const captureId = id.describe('studio_capture_state 或 studio_capture_set_plan 返回的连续拍计划 ID。')
const captureText = (max: number, description: string) => z.string().trim().min(1).max(max).describe(description)
const captureAssetIds = z.array(id).max(10).describe('计划内资产 ID，按参考图顺序排列。')
const capturePlan = z.strictObject({
  title:captureText(120,'连续拍标题。'),
  aesthetic:captureText(400,'全片画风和材质。'),
  assets:z.array(z.strictObject({
    id:captureText(80,'计划内唯一的资产 ID。'),
    name:captureText(120,'资产显示名称。'),
    kind:z.enum(['character','prop','scene']).describe('资产类型。'),
    prompt:captureText(8000,'直接用于资产生图的最终正文。')
  })).max(100).describe('本计划复用的资产，可为空。'),
  scenes:z.array(z.strictObject({
    id:captureText(80,'计划内唯一的场次 ID。'),
    name:captureText(120,'场次名称。'),
    sourceText:captureText(8000,'该场对应的剧本文本。'),
    openingPrompt:captureText(8000,'该场单幅开场图的生图正文。'),
    assetIds:captureAssetIds,
    segments:z.array(z.strictObject({
      title:captureText(120,'子镜头标题。'),
      duration:z.number().min(6).max(30).describe('子镜头时长（秒），须符合对应工作流。'),
      prompt:captureText(8000,'子镜头最终视频提示词，原样进入工作流。'),
      startState:captureText(2000,'可见起幅状态。'),
      endState:captureText(2000,'可见落幅状态。'),
      sound:captureText(2000,'声音与环境内容。'),
      referenceAssetIds:captureAssetIds.optional().describe('本镜工作流中普通参考图的槽位顺序，不包含首帧。'),
      dialogue:z.array(z.strictObject({
        speaker:captureText(80,'说话人。'), text:captureText(2000,'剧本对白原文。'), delivery:captureText(200,'语气或表演。')
      })).max(40).optional().describe('对白记录，供分镜稿审读。最终台词同时写进视频正文。')
    })).min(1).max(100).describe('同场子镜头按真实拍摄顺序排列。')
  })).min(1).max(100).describe('按拍摄顺序排列的场次。')
}).describe('结构化连续拍计划。写入只建画布，不产生生成任务。')

const shotSelector = id.describe('镜头 ID，或当前项目全部分镜中的一基镜号，如 #05。按项目全局顺序，筛选不会重新编号。')
const shotInputFields = {
  videoProfileId: id.nullable().optional().describe('studio_list_models 的视频配置 ID。仅用于此镜。null 清除镜头指定配置。'),
  duration: z.number().positive().max(120).optional().describe('镜头时长，单位为秒。工作流可能有更窄的限制，提交前需预检。'),
  audioMode: z.enum(['none','tts','h3-dialogue','h3-voiceover']).optional().describe('无口播、独立配音、出镜对白或画外音'),
  subjectBindingMode: z.enum(['infer','explicit']).optional().describe('infer 可按文字补全角色，explicit 仅用显式人物 ID'),
  subjectIds: z.array(id).max(24).optional().describe('人物卡 ID，空数组在 explicit 模式表示本镜无人'),
  sceneId: id.nullable().optional().describe('场景卡 ID。省略保留，null 明确解绑场景并停止自动补全。'),
  propIds: z.array(id).max(24).nullable().optional().describe('道具卡 ID。省略保留，空数组明确不绑定道具，null 恢复按文案推断。'),
  videoStillMode: z.enum(['identity','keyframe','storyboard']).optional().describe('人物场景参考、独立关键帧或故事看板。需有分镜视频节点'),
  h3GenMode: z.enum(['reference','fl2va']).optional().describe('画布输入模式。不填时跟随项目默认（项目首尾帧链）；填了就按这镜自己的来。MCP 出片预检另选工作流输入模式'),
  firstFrameAssetId: id.nullable().optional().describe('项目首帧图片 ID，null 清除'),
  lastFrameAssetId: id.nullable().optional().describe('项目尾帧图片 ID，null 清除')
}
const comfyRatio = z.enum(['1:1','2:3','3:2','3:4','4:3','9:16','16:9','21:9'])
const comfyAspect = comfyRatio.optional().describe('选择模型 outputConfig 声明的画幅。省略时沿用镜头或项目画幅。')
const referenceAssignments = z.array(z.strictObject({
  index: z.number().int().min(1).max(10).describe('referenceConfig 中的槽位序号，从 1 开始。'),
  assetId: id.describe('要放入此槽的项目图片素材 ID。')
})).max(10).optional().describe('按声明的槽位直接赋值，可跳过可选槽。和 referenceAssetIds 二选一。')
const referenceConfig = z.strictObject({
  mode: z.literal('ordered-optional').describe('按声明顺序分配，缺少的可选槽按 fallback 处理。'),
  slots: z.array(z.strictObject({
    index: z.number().int().min(1).max(10).describe('从 1 连续编号，对应 referenceAssetIds 传入顺序。'),
    nodeId: id.describe('普通参考图的 LoadImage 节点，首尾帧沿用专用输入。'),
    field: z.literal('image').optional().describe('普通输入的字段名，默认 image。'),
    role: z.enum(['identity','storyboard','scene','prop','reference']).describe('人物、动作看板、场景、道具或通用参考。'),
    required: z.boolean().describe('是否需要真实的项目素材。'),
    fallback: z.literal('neutral').optional().describe('仅可选槽使用。留空时使用指定的项目中性图。')
  })).max(10).describe('声明所有普通参考图入口，从 1 顺序编号。更新时完整替换这个数组。'),
  neutralAssetId: id.optional().describe('当前项目内已归档的中性图片 ID，供可选槽补位。'),
  boardInFrameSlot: z.literal('block').optional().describe('首尾帧拦截已标记的分格看板。')
})
const outputConfig = z.strictObject({
  aspectRatios: z.array(comfyRatio).min(1).max(8).optional().describe('允许选择的画幅。须有实际工作流输入支持。'),
  defaultAspectRatio: comfyRatio.optional().describe('镜头和项目均无画幅时使用的声明默认值。'),
  resolutionByRatio: z.record(z.string(),z.string().regex(/^\d+x\d+$/).nullable()).optional()
    .describe('每种画幅的目标宽x高。有宽高映射时写精确像素，ResolutionSelector 则写目标像素量，实际尺寸可能取整。null 删除指定项。'),
  durationNode: id.optional().describe('视频秒数对应的节点 ID。可使用现有时长映射或在这里明确指定。'),
  durationField: id.optional().describe('该节点的数字秒数字段，存在多个候选时必须指定。'),
  durationRange: z.tuple([z.number().positive().max(120),z.number().positive().max(120)]).optional().describe('视频时长下限和上限，单位秒。')
})
const videoTake = z.strictObject({
  shotId: id.describe('普通镜头用 studio_shots 的分镜 ID，连续拍用 studio_capture_state 的视频 nodeId。'),
  mode: z.enum(['reference', 'first-frame', 'first-last-frame', 'previous-tail', 'text']).describe('生成输入模式。reference 普通参考图，first-frame 首帧，first-last-frame 首尾帧，previous-tail 同场接上一镜已选成片，text 纯提示词。按实际工作流选择。'),
  profileId: id.optional().describe('已保存的模型接入配置 ID，用 studio_list_models 查询。区别于供应商的模型名。'),
  referenceAssetIds: z.array(id).max(9).optional().describe('普通参考图按工作流槽位顺序填写，可与首帧一起提交。首尾帧另填对应字段。'),
  allowTemplateImages: z.literal(true).optional().describe('true 会把预检列出的模板固定图片保留在提交的工作流中，模型会实际使用它们。仅在用户确认这些具体文件后填写。省略时有未绑定的固定图就拦截。'),
  bindingReview, referenceAssignments, aspectRatio:comfyAspect,
  firstFrameAssetId: id.optional().describe('当前镜头起始画面的图片素材 ID。用于首帧模式；previous-tail 由工作台从上一镜选中成片自动取得。'),
  lastFrameAssetId: id.optional().describe('当前镜头结束画面的图片素材 ID，仅在需要提供尾帧的工作流使用。')
})
const seconds = z.number().min(0).max(86400)
const planId = id.describe('项目内稳定的执行计划 ID。')
const imageLayout = z.strictObject({
  title: z.string().trim().min(1).max(160).optional().describe('供项目和预检定位的标题，不写进模型提示词。'),
  aspectRatio: z.enum(['16:9','9:16','1:1','4:3','21:9']).optional().describe('本次制作意图的画幅。这里只记录，真正画幅由工作流决定。'),
  rows: z.number().int().min(1).max(6).optional().describe('预期分格行数，和列数一起填。'),
  columns: z.number().int().min(1).max(6).optional().describe('预期分格列数，和行数一起填。')
}).optional().describe('给人和 Agent 查看、不会送入模型的版式资料。')
const imagePromptFields = {
  visualPrompt: z.string().trim().min(1).max(12000).describe('送模型的纯视觉正文，标题、画幅和版式资料另存。'),
  promptLayout: imageLayout.unwrap().nullable().optional().describe('版式资料。省略保留，null 清除。'),
  syncScript: z.literal(true).optional().describe('仅镜头关键帧。true 同步覆盖分镜 visualPrompt，省略保留分镜正文。')
}
const shotBatchEdit = z.discriminatedUnion('kind',[
  z.strictObject({ kind:z.literal('script-prompt').describe('只修改分镜稿的画面正文'), shotId:shotSelector, visualPrompt:shotDraft.shape.visualPrompt }),
  z.strictObject({ kind:z.literal('image-prompt').describe('修改镜头关键帧正文'), shotId:shotSelector, ...imagePromptFields }),
  z.strictObject({ kind:z.literal('video-prompt').describe('保存视频最终稿'), shotId:shotSelector, prompt:z.string().trim().min(1).max(12000).describe('提交给视频模型的最终正文') }),
  z.strictObject({ kind:z.literal('inputs').describe('更新镜头输入设置'), shotId:shotSelector, ...shotInputFields }),
  z.strictObject({
    kind: z.literal('shot-fields').describe('更新镜头编排、表演稿或审阅字段。省略字段保留当前值。'),
    shotId: shotSelector,
    title: shotDraft.shape.title.optional(),
    cameraMovement: shotDraft.shape.cameraMovement.optional(),
    shotSize: z.string().max(160).nullable().optional().describe('镜头景别；null 清除。'),
    blocking: z.string().max(4000).nullable().optional().describe('人物位置、朝向和调度；null 清除。'),
    eyeline: z.string().max(4000).nullable().optional().describe('人物视线与面对关系；null 清除。'),
    continuity: z.string().max(4000).nullable().optional().describe('与相邻镜头的初态、末态及衔接；null 清除。'),
    purpose: z.string().max(4000).nullable().optional().describe('镜头的叙事目的；null 清除。'),
    propFocus: z.string().max(4000).nullable().optional().describe('镜头的道具焦点与状态；null 清除。'),
    audioDescription: z.string().max(4000).optional().describe('镜内环境声与动作声；空字符串清除。'),
    videoPrompt: z.string().max(12000).nullable().optional().describe('镜头表演稿；null 清除。出片终稿独立保存。'),
    firstFramePrompt: z.string().max(12000).nullable().optional().describe('单幅首帧正文；null 清除。'),
    lastFramePrompt: z.string().max(12000).nullable().optional().describe('单幅尾帧正文；null 清除。'),
    cutIn: z.enum(['action-start','mid-action','after-action','on-reaction']).nullable().optional().describe('分镜审阅用的切入拍点；null 清除。'),
    audienceGain: z.string().max(4000).nullable().optional().describe('分镜审阅时观众获得的新信息；null 清除。')
  })
])
const planStepBase = {
  id: id.describe('计划内唯一的步骤 ID。'),
  nodeId: id.optional().describe('用于在画布定位此步骤的节点 ID。'),
  sources: z.array(z.strictObject({
    stepId: id.describe('提供所需媒体类型的上游步骤 ID。'),
    slot: z.string().regex(/^(firstFrame|lastFrame|video|reference:[0-9])$/).describe('首帧 firstFrame、尾帧 lastFrame、提取尾帧所需 video，或从 0 开始的 reference:0 等槽位。')
  })).max(12).optional().describe('以步骤输出接入素材槽，搭建时可以没有素材。'),
  after: z.array(id).max(500).optional().describe('必须成功归档的前置步骤 ID。previous-tail 显式依赖上一镜步骤；导出可在此依赖需要的镜头。')
}
const planReferences = z.array(id.nullable()).max(10).optional().describe('已有参考图素材 ID，按工作流槽位排序。由 sources 绑定的同位置可用 null 占位。')
const volume = z.number().min(0).max(1)
const clip = z.strictObject({
  id: id.describe('该条记录的稳定 ID。新建需唯一；更新已有记录时使用读回的 ID。'), shotId: id.describe('分镜 ID，由 studio_shots 或写入结果返回。请使用 ID，避免把显示镜号误作 ID。'), assetId: id.optional().describe('项目素材库中的素材 ID，可用 studio_project_items 的 assets 查询。按用途选择图片、视频或音频。'), label: z.string().max(200).optional().describe('在界面展示的简短标签。'),
  startTime: seconds.describe('在成片时间线中的起始位置，单位为秒。'), duration: seconds.positive().describe('持续时长，单位为秒。'), sourceIn: seconds.describe('从源素材的哪一秒开始截取。'),
  transitionIn: z.enum(['none', 'fade', 'fade-black', 'slide-left', 'slide-right', 'zoom-in', 'zoom-out']).describe('片段入场转场类型，none 表示无转场。'),
  transitionDuration: seconds.describe('入场转场持续时间，单位为秒。'), speed: z.number().min(0.1).max(8).describe('剪辑播放倍率。1 为原速，2 为两倍速。'), volume: volume.describe('音量比例，0 静音，1 原音量。'), enabled: z.boolean().describe('是否将此片段纳入播放和导出。'),
  voiceAssetId: id.optional().describe('与这个片段关联的配音素材 ID。'), voiceSourceIn: seconds.optional().describe('配音素材的截取起点，单位为秒。'), voiceVolume: volume.optional().describe('配音音量，0 静音，1 原音量。')
})
const planStep = z.discriminatedUnion('kind',[
  z.strictObject({...planStepBase,kind:z.literal('image').describe('服务端生图，支持原生 ComfyUI 和 OpenAI Images。'),input:z.strictObject({
    ...generationImageTarget, profileId:videoTake.shape.profileId,
    visualPrompt:z.string().trim().min(1).max(12000).optional().describe('保存到计划的纯视觉正文。省略时用资产卡或镜头保存的提示词。'),
    promptLayout:imageLayout,bindingReview,referenceAssignments,aspectRatio:comfyAspect,referenceAssetIds:planReferences,outputNodeId:id.optional().describe('指定读取图片的 ComfyUI 输出节点。')
  }).describe('生图目标、提示词和实际输入。')}),
  z.strictObject({...planStepBase,kind:z.literal('video').describe('逐镜生成视频。'),input:z.strictObject({
    ...videoTake.shape, referenceAssetIds:planReferences, extensionNodeId:id.optional().describe('可选的 ComfyUI 视频扩展节点 ID。')
  }).describe('本镜生成模式和工作流输入。')}),
  z.strictObject({...planStepBase,nodeId:id.describe('已放到画布上的视频尾帧提取节点 ID。'),kind:z.literal('tail').describe('提取视频尾帧。'),
    input:z.strictObject({}).describe('传空对象。使用 sources 绑定上游视频，或使用尾帧节点已配置的输入。')}),
  z.strictObject({...planStepBase,kind:z.literal('export').describe('基础硬切导出。'),input:z.strictObject({
    clips:z.array(clip).max(500).optional().describe('可先保存不含素材的时间线，after 中相应视频步骤归档后自动补入。'),
    buildFromShots:z.literal(true).optional().describe('按现有分镜顺序组装。与 clips 二选一，均省略时使用项目时间线。')
  }).describe('导出需要的时间线参数。')})
])
const bgm = z.strictObject({ assetId: id.describe('项目素材库中的素材 ID，可用 studio_project_items 的 assets 查询。按用途选择图片、视频或音频。'), startTime: seconds.describe('在成片时间线中的起始位置，单位为秒。'), sourceIn: seconds.describe('从源素材的哪一秒开始截取。'), duration: seconds.positive().describe('持续时长，单位为秒。'), volume: volume.describe('音量比例，0 静音，1 原音量。') })
const overlay = z.strictObject({
  id: id.describe('该条记录的稳定 ID。新建需唯一；更新已有记录时使用读回的 ID。'), text: z.string().max(4000).describe('直接显示在画面上的文字，不含时间或样式标记。'), startTime: seconds.describe('在成片时间线中的起始位置，单位为秒。'), duration: seconds.positive().describe('持续时长，单位为秒。'),
  fontFamily: z.string().max(200).describe('叠加文字的字体名称。'), fontSizeRatio: z.number().positive().max(1).describe('字号相对画面高度的比例，范围大于 0 且不超过 1。'), color: z.string().regex(/^#[0-9a-f]{6}$/i).describe('文字颜色，使用六位十六进制 RGB，例如 #FFFFFF。'),
  position: z.enum(['top', 'center', 'bottom']).describe('叠加文字的垂直预设位置：顶部、居中或底部。'), xRatio: volume.optional().describe('自定义横向位置，0 左端，1 右端。'), yRatio: volume.optional().describe('自定义纵向位置，0 顶端，1 底端。'), bold: z.boolean().describe('文字叠加是否使用粗体。')
})
const adapterType = z.enum(['openai-compatible', 'openai-images', 'http-custom', 'comfyui-workflow', 'zealman-workflow', 'comfyui-prompt'])
const comfyTarget = z.strictObject({
  root: z.string().trim().min(1).max(2048).optional().describe('发现结果里的安装目录。可省略，直接浏览在线端口。'),
  port: z.number().int().min(1).max(65535).optional().describe('发现结果里的本机回环端口。')
}).refine(input => input.root || input.port, '请选择安装目录或端口')
const comfyBrowse = {
  instanceId: id.optional().describe('已保存实例的 ID。与 target 二选一。'),
  target: comfyTarget.optional().describe('studio_comfy_discover 返回的 target。临时查看和导入无需保存实例。')
}
const hasComfySource = (input: { instanceId?: string; target?: object }) => Boolean(input.instanceId) !== Boolean(input.target)
const extensionParamValue = z.union([z.string().max(4096), z.number(), z.boolean()])
const extensionPort = z.strictObject({
  id: id.describe('端口稳定 ID，例如 subject。'), kind: z.enum(['image', 'video']).describe('端口传输的素材类型。')
})
const extensionManifest = z.strictObject({
  formatVersion: z.literal(1).describe('清单格式版本。'),
  id: id.describe('用户扩展的命名空间 ID，例如 drama.h3。'),
  version: z.number().int().positive().describe('节点类型版本。修改同一类型的定义时递增。'),
  name: id.describe('画布显示名称。'), description: z.string().min(1).max(500).describe('节点用途和输入说明。'),
  executor: z.enum(['studio.video-tail', 'studio.comfy-video', 'studio.mcp']).describe('尾帧提取、ComfyUI 视频或 MCP 工具。'),
  profileId: id.optional().describe('ComfyUI 视频配置 ID。'),
  providerId: id.optional().describe('MCP 连接 ID。'), tool: id.optional().describe('该连接中允许调用的工具名。'),
  outputKind: z.literal('image').optional().describe('MCP 扩展声明图片输出时填写 image。'),
  params: z.array(z.strictObject({
    key: id.describe('参数稳定键，例如 steps。'), label: id.describe('画布表单标签。'),
    type: z.enum(['string', 'integer', 'number', 'boolean', 'model']).describe('值类型。model 是 ComfyUI 权重文件名。'),
    default: extensionParamValue.optional().describe('无实例覆盖时使用的值。省略时保留工作流原值。'),
    enum: z.array(extensionParamValue).max(100).optional().describe('限定可选值。ComfyUI 普通字符串须声明枚举。'),
    min: z.number().optional().describe('数值最小值。'), max: z.number().optional().describe('数值最大值。'),
    target: z.strictObject({ nodeId: id.describe('API 图节点 ID。'), field: id.describe('目标节点的 inputs 字段。') }).optional()
      .describe('ComfyUI 参数绑定到 API 图中的节点和输入字段。')
  })).max(32).optional().describe('节点实例可覆盖的参数。'),
  inputs: z.array(extensionPort).max(10).optional().describe('ComfyUI 图片端口按声明顺序送入工作流的普通参考图槽。'),
  outputs: z.array(extensionPort).max(1).optional().describe('指定执行器支持的输出端口 ID 与类型。')
}).describe('自定义节点类型清单。')

const videoPromptFields = { shotId:shotSelector, prompt:z.string().trim().min(1).max(12000)
  .describe('直接发送给视频模型的最终正文，写清动作、环境、运镜和连续性。') }
const assetPromptFields = { ...imageTarget, targetId:id.describe('资产卡 ID；镜头可用 #05。'), ...imagePromptFields }
const shotInputs = { shotId:shotSelector, ...shotInputFields }
const editable = <S extends z.ZodRawShape>(shape: S) => {
  const item = z.strictObject(shape)
  return z.strictObject({
    ...revision, ...item.partial().shape,
    items:z.array(item).min(1).max(100).optional().describe('批量模式按顺序提交 1 至 100 项，任何一项失败时整批不写。')
  }).refine((input) => {
    const single = Object.fromEntries(Object.entries(input).filter(([key]) =>
      key !== 'projectId' && key !== 'expectedUpdatedAt' && key !== 'items'))
    return (input as Record<string, unknown>).items === undefined ? item.safeParse(single).success : Object.keys(single).length === 0
  }, '请选择完整的单项字段或 items 批量模式')
}

export const studioTools = {
  studio_discard_video_plan: { description: '撤销尚未提交的已预检视频批次，保留记录，不调用模型。', schema: z.strictObject(videoJob) },
  studio_discard_image_plan: { description: '撤销尚未提交的已预检生图计划，保留记录，不调用模型。', schema: z.strictObject(videoJob) },
  studio_comfy_discover: { description: '只读扫描本机 ComfyUI 安装和常见回环端口。返回可直接浏览的 target，无需保存实例。', schema: z.strictObject({
    ports: z.array(z.number().int().min(1).max(65535)).max(20).optional().describe('只探测这些回环端口。省略时探测常用和已保存的端口。'),
    roots: z.array(z.string().trim().min(1).max(2048)).max(20).optional().describe('指定要扫描的绝对目录数组。省略时扫描常用安装位置；不会自动启动实例。')
  }) },
  studio_comfy_instances: { description: '查看本工作区已登记的 ComfyUI 实例及在线状态。', schema: z.strictObject({}) },
  studio_comfy_register: { description: '登记用户选中的 ComfyUI 安装目录和端口。仅填写端口时，接入现有服务。', schema: z.strictObject({
    root: z.string().trim().min(1).max(2048).optional().describe('ComfyUI 安装目录绝对路径，可填写带 ComfyUI 子目录的便携包根目录。'), port: z.number().int().min(1).max(65535).optional().describe('ComfyUI 回环端口。选择检测到的端口，或为该安装配置独立端口，例如 8189。'),
    name: z.string().trim().min(1).max(100).optional().describe('工作台内用于区分该 ComfyUI 实例的名称。省略时按端口命名。'), confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。')
  }).refine(input => input.root || input.port, '请输入安装目录或端口') },
  studio_comfy_start: { description: '启动用户确认的本机 ComfyUI 实例，复用已在线的端口。', schema: z.strictObject({ instanceId: id.describe('已登记的 ComfyUI 实例 ID，由 studio_comfy_instances 或 studio_comfy_register 返回。'), confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') }) },
  studio_comfy_stop: { description: '停止由工作台启动且队列为空的 ComfyUI 实例，需要用户确认。', schema: z.strictObject({ instanceId: id.describe('已登记的 ComfyUI 实例 ID，由 studio_comfy_instances 或 studio_comfy_register 返回。'), confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') }) },
  studio_comfy_remove: { description: '移除已确认的实例登记，保留源安装和已导入工作流。', schema: z.strictObject({ instanceId: id.describe('已登记的 ComfyUI 实例 ID，由 studio_comfy_instances 或 studio_comfy_register 返回。'), confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') }) },
  studio_comfy_workflows: { description: '浏览 ComfyUI 工作流。唯一在线实例可直接传空对象，有多个时传 discover 返回的 target 或 instanceId。每条结果的 readInput 可直接传给 studio_comfy_workflow_read 或 studio_comfy_import。', schema: z.strictObject(comfyBrowse).refine(input => !(input.instanceId && input.target), '实例 ID 和发现目标只能传一个') },
  studio_comfy_workflow_read: { description: '读取实例工作流的节点、输入和权重。需要时自动取得 API JSON，供 Dramlo 或其他工作台导入。', schema: z.strictObject({
    ...comfyBrowse, source: z.enum(['disk', 'server']).describe('沿用工作流列表中的来源。disk 安装目录文件，server 在线实例保存的文件。'), folder: z.number().int().min(0).optional().describe('文件来自 disk 时，填写工作流列表返回的目录序号，从 0 开始。server 来源可省略。'),
    relativePath: z.string().min(1).max(2048).describe('工作流列表返回的相对路径，保留目录和 .json 后缀，勿填写绝对路径。'), includeApiJson: z.boolean().optional().describe('填 true 返回可执行的 API JSON 字符串；只看节点与输入摘要时可省略。'),
    includeNodeInputs:z.literal(true).optional().describe('按实际实例的 object_info 读取节点输入类型、枚举、范围、单位及推测的映射。')
  }).refine(hasComfySource, '请从实例 ID 和发现目标中选一项') },
  studio_comfy_import: { description: '从选定实例自动取得 API 图并创建图片或视频配置。界面图使用该实例前端序列化；不会提交生成。', schema: z.strictObject({
    ...comfyBrowse, source: z.enum(['disk', 'server']).describe('沿用工作流列表中的来源。disk 安装目录文件，server 在线实例保存的文件。'), folder: z.number().int().min(0).optional().describe('文件来自 disk 时，填写工作流列表返回的目录序号，从 0 开始。server 来源可省略。'),
    relativePath: z.string().min(1).max(2048).describe('工作流列表返回的相对路径，保留目录和 .json 后缀，勿填写绝对路径。'), kind: z.enum(['image', 'video']).optional().describe('可选。image 登记为生图接入，video 登记为视频接入。省略时根据输出节点自动识别。'),
    name: z.string().trim().min(1).max(120).optional().describe('在模型列表显示的工作流名称。省略时沿用文件名。'), select: z.literal(true).optional().describe('填写 true 将已导入且映射完整的工作流选为全局当前接入。省略时只新增。'),
    dryRun:z.literal(true).optional().describe('只返回导入候选、槽位与源图指纹，无配置写入。'),
    workflowFingerprint:id.optional().describe('使用 dryRun 返回的指纹，确认导入预览过的工作流。'),
    referenceConfig:referenceConfig.optional().describe('同时声明已核对的普通图片槽语义。'),
    outputConfig:outputConfig.optional().describe('同时声明画幅、分辨率和时长约束。')
  }).refine(hasComfySource, '请从实例 ID 和发现目标中选一项') },
  studio_create_project: { description: '创建并打开一个工作台项目。', schema: z.strictObject({ name: z.string().trim().min(1).max(100).describe('用户在界面看到的名称。填写有辨识度的实际名称。') }) },
  studio_delete_project: { description: '预览或移除指定项目。确认后将项目目录移到工作区的可恢复目录。当前项目及有未结提交的项目会被拦截。', schema: z.strictObject({
    projectId: revision.projectId, expectedUpdatedAt: exactRevision.optional().describe('确认移除时填写预览返回的项目版本。'),
    confirmed: z.literal(true).optional().describe('省略时预览，获得用户针对该项目的明确确认后才传 true。')
  }) },
  studio_rename_project: { description: '按项目修订号更新项目显示名称，保留目录和素材。', schema: z.strictObject({ ...revision, name: z.string().trim().min(1).max(100).describe('用户在界面看到的名称。填写有辨识度的实际名称。') }) },
  studio_list_projects: { description: '分页列出本机项目。', schema: z.strictObject({ offset: page.offset.describe('分页起点，从 0 开始。继续读取时使用上一次返回的 nextOffset。'), limit: page.limit.describe('本页最大条目数。') }) },
  studio_open_project: { description: '按项目 ID 打开已保存的项目。', schema: z.strictObject({ projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。') }) },
  studio_project_items: { description: '分页读取人物、场景、道具、章节、素材、画布节点或连线。', schema: z.strictObject({
    ...optionalPage, type: z.enum(['subjects', 'scenes', 'props', 'chapters', 'assets', 'nodes', 'edges']).describe('要读取的项目数据类型。')
  }) },
  studio_set_video_prompt: {
    description: '将最终提示词写入 compiledVideoPrompt。原生视频队列提交时将它直接写入工作流提示词槽。返回镜内拍点与时长提醒，保留已有选片，不提交生成。',
    schema: editable(videoPromptFields)
  },
  studio_prepare_video: {
    description: '检查一镜或一批分镜的 compiledVideoPrompt、工作流和实际素材槽，保存按分镜顺序排列的预检计划。返回项目参考、实际输入与预计工作量，不上传或生成。',
    schema: z.strictObject({ ...videoJob, expectedUpdatedAt: autoRevision, shots: z.array(videoTake).min(1).max(30).describe('待预检的镜头及素材绑定。每镜填写已有 shotId、生成模式和该模式使用的素材。') })
  },
  studio_confirm_video: {
    description: '用户明确批准后按镜头顺序启动预检计划。每镜归档后才开始下一镜。首次调用可返回运行中，完成后用 studio_video_status 查看每镜 bind。复用 requestId 不会重复提交。',
    schema: z.strictObject({ ...videoJob, confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') })
  },
  studio_current_video_batch: {
    description: '查看活动项目最近开始的制作批次和待恢复状态。',
    schema: z.strictObject({ projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。') })
  },
  studio_video_status: {
    description: '查询批次、每镜任务 ID、错误和已归档视频。已保存步骤的 bind.applied 表示归档时是否自动选用，bind.current 和 bind.isCurrent 表示现在的镜头选片。旧任务的 applied 可能为 null。',
    schema: z.strictObject(videoJob)
  },
  studio_pause_video: {
    description: '当前镜头完成并保存后暂停本批，独立 ComfyUI 继续运行。',
    schema: z.strictObject(videoJob)
  },
  studio_resume_video: {
    description: '继续已确认的暂停批次，或查询已有远端任务 ID 并继续。请先让用户确认，无法确定是否已提交的镜头会继续阻止重试。',
    schema: z.strictObject({ ...videoJob, confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') })
  },
  studio_recover_video_id: {
    description: '核对未知提交的原任务 ID。使用历史/队列里的独立标识验证归属，不会重新提交。旧 ComfyUI 可传入人工找到的 promptId。',
    schema: z.strictObject({ ...videoJob, promptId: id.optional().describe('ComfyUI 原任务的 prompt_id。如已知道，填写从历史或队列核实的任务 ID。') })
  },
  studio_cancel_video_shot: {
    description: '向 ComfyUI 定向取消正在生成的镜头并暂停后续。用户核对具体任务后，传该镜的 expectedPromptId 可防止取消已接续的镜头。',
    schema: z.strictObject({ ...videoJob, expectedPromptId: id.optional().describe('用户确认停止的 ComfyUI 任务 ID，从等待状态镜头的 promptId 读取。'),
      confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') })
  },
  studio_stop_video_wait: {
    description: '停止本批的本地等待和后续提交。ComfyUI 已接收的任务仍可完成，用原 requestId 恢复查询。',
    schema: z.strictObject(videoJob)
  },
  studio_locate_asset: {
    description: '查询当前项目素材的本地原片路径和存在状态。',
    schema: z.strictObject({ projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), assetId: id.describe('项目素材库中的素材 ID，可用 studio_project_items 的 assets 查询。按用途选择图片、视频或音频。') })
  },
  studio_bind_shot_asset: { description: '从项目素材中选择镜头使用的看板、主图、视频、配音、首帧或尾帧。返回绑定影响与主图和首帧的一致状态。image 只改主图，first 才修改首帧槽。原文件保留。首尾帧只接受单幅图。', schema: z.strictObject({
    ...revision, shotId: id.describe('分镜 ID，由 studio_shots 或写入结果返回。请使用 ID，避免把显示镜号误作 ID。'), assetId: id.describe('项目素材库中的素材 ID，可用 studio_project_items 的 assets 查询。按用途选择图片、视频或音频。'), slot: z.enum(['board', 'image', 'video', 'audio', 'first', 'last']).describe('镜头素材槽。board 看板、image 关键帧、video 视频、audio 配音、first 首帧、last 尾帧。')
  }) },
  studio_import_script: {
    description: '导入原文及可选章节到没有剧本的项目。保留原文，已有剧本需通过专门的修改入口。',
    schema: z.strictObject({
      ...revision, title: id.describe('整部剧本标题。用简短主题或片名，例如「修仙渡劫」。章节标题另填在 chapters 内。'), content: z.string().min(1).max(50000).describe('完整剧本或章节正文。保留场次、动作、角色和对白，按原文段落提供。'),
      chapters: z.array(z.strictObject({ id: z.uuid().describe('该条记录的稳定 ID。新建需唯一；更新已有记录时使用读回的 ID。'), title: id.describe('该章标题，例如「第一场：雨夜归来」。'), content: z.string().min(1).max(50000).describe('完整剧本或章节正文。保留场次、动作、角色和对白，按原文段落提供。') })).max(25).optional().describe('章节列表，数组顺序即剧本顺序。已有完整章节的剧本可提供标题与原文；无章节可省略。')
    })
  },
  studio_upsert_project_assets: {
    description: '创建或修改人物、场景、道具卡片文本。保留原有图片、选用版本和历史。',
    schema: z.strictObject({
      ...revision,
      items: z.array(z.discriminatedUnion('type', [
        z.strictObject({ ...projectAsset, type: z.literal('subject').describe('要读取或写入的项目数据类型。'), promptName:z.string().trim().min(1).max(80).nullable().optional().describe('人物在生图或视频提示词里使用的固定名称，例如英文名。省略保留，null 清除。'), kind: z.enum(['person', 'animal', 'other']).describe('主体分类：person 人物、animal 动物、other 其他拟人或生物主体。') }),
        z.strictObject({ ...projectAsset, type: z.literal('scene').describe('要读取或写入的项目数据类型。') }),
        z.strictObject({ ...projectAsset, type: z.literal('prop').describe('要读取或写入的项目数据类型。') })
      ])).min(1).max(25).describe('人物、场景或道具条目。按 type 填写 ID、名称和可复用的外观描述。')
    })
  },
  studio_remove_project_asset: {
    description: '预览或移除未被项目或任务引用的已归档素材或角色卡。移除素材时将文件移入工作区可恢复目录。移除角色卡时保留原始角色板图片。',
    schema: z.strictObject({
      projectId: page.projectId,
      assetId: id.describe('要移除的素材条目 ID 或角色卡 ID，从 studio_project_items 查询。'),
      targetType: z.enum(['media','subject']).optional().describe('media 是项目素材，subject 是角色卡。默认 media。'),
      expectedUpdatedAt: exactRevision.optional().describe('先预览，再将响应中的 updatedAt 作为数字传回。确认移除时必填。'),
      confirmed: z.literal(true).optional().describe('省略只预览。有用户确认并核对引用后传 true。')
    })
  },
  studio_update_script: { description: '局部更新剧本。只改标题、关键词或风格时保留正文与确认状态；正文改变时重置剧本确认。', schema: z.strictObject({
    ...revision,
    content: z.string().max(50000).refine(value => value.trim().length > 0, '正文不能为空').optional().describe('要替换的完整剧本正文，仅在确实修改正文时提供。只改元数据时省略。'),
    topic: id.optional().describe('剧本主题或显示标题。例如「修仙渡劫短剧」；剧情本身填写 content。'), keywords: z.array(id).max(50).optional().describe("主题或风格关键词数组，例如 [\"夜雨\",\"悬疑\"]。传空数组清空，省略则保留。"),
    style: z.enum(['cinematic', 'anime', 'oil-painting', '3d-render', 'photorealistic', 'watercolor', 'pixel-art', 'ink-wash']).optional().describe('剧本的画面风格，如 cinematic 电影感、anime 动漫、photorealistic 写实。省略时保留。')
  }).refine(input => input.content !== undefined || input.topic !== undefined ||
    input.keywords !== undefined || input.style !== undefined, '请提供要更新的剧本字段') },
  studio_edit_timeline: { description: '保存时间线，可从分镜自动组装或提交完整片段列表，也可设置字幕、配乐和文字叠加。null 清除配乐。', schema: z.strictObject({
    ...revision, clips: z.array(clip).max(500).optional().describe('完整的剪辑片段列表，按播放顺序填写。提供后替换现有时间线，与 buildFromShots 互斥。'), buildFromShots: z.literal(true).optional().describe('填 true 使用已有分镜素材自动组装时间线。不能同时传 clips。'),
    captionSrt: z.string().max(50000).optional().describe('完整 SRT 文本，含序号、起止时间和字幕。省略时保留旧字幕。'), bgm: bgm.nullable().optional().describe('背景音乐设置，包含素材 ID、起点和音量等。传 null 移除，省略则保留。'), overlays: z.array(overlay).max(250).optional().describe('完整叠加文字列表，包含显示区间、位置和样式。省略时保留现有内容。')
  }).refine((input) => !(input.clips && input.buildFromShots) &&
    (input.clips !== undefined || input.buildFromShots || input.captionSrt !== undefined || input.bgm !== undefined || input.overlays !== undefined),
  '请提供修改内容，手工片段与自动组装只能选一项') },
  studio_set_credential: { description:'向指定模型配置写入或清除用户提供的 API 密钥。响应只返回凭据是否存在。调用方应避免把密钥写进日志和文档。', schema:z.strictObject({
    kind:kind.describe('配置所属模型类型。'), profileId:id.describe('studio_list_models 返回的配置 ID。'),
    expectedBaseUrl:endpoint.describe('用户核对过的完整服务地址，须与保存的配置一致。在线地址须用 HTTPS。'),
    apiKey:z.string().min(1).max(8192).nullable().describe('用户授权写入的 API 密钥。传 null 清除该配置的 API 密钥。')
  }) },
  studio_patch_workflow: { description:'按节点修改现有工作流的文本、数字或布尔输入。保持原有节点连线、映射和默认配置选择。', schema:z.strictObject({
    kind:z.enum(['image','video']).describe('原生工作流所属类型。'), profileId:id.describe('要修改的模型配置 ID。'),
    expectedWorkflowRevision:id.describe('studio_describe_model 返回的 workflowRevision，避免覆盖其他人的修改。'),
    patches:z.array(z.strictObject({
      nodeId:id.describe('API 图内的节点 ID。'),field:id.describe('已存在的 inputs 字段名。'),
      value:z.union([z.string().max(80000),z.number(),z.boolean()]).describe('要写入的新值，与原输入的标量类型一致。')
    })).min(1).max(64).describe('一次提交的修改列表，同一节点字段只能出现一次。')
  }) },
  studio_model_update: { description: '按 ID 修改模型配置，保留当前默认项。视频的 model 为工作流 ID。局部改工作流用 studio_patch_workflow，密钥用 studio_set_credential。', schema: z.strictObject({
    kind: kind.describe('接入类型。llm 文本、vision 视觉理解、image 图像、video 视频、tts 配音、music 音乐。'), profileId: id.describe('已保存的模型接入配置 ID，用 studio_list_models 查询。区别于供应商的模型名。'), model: id.optional().describe('供应商的模型标识；对视频接入，按该适配器填写模型或工作流 ID。'), baseUrl: endpoint.optional().describe('服务根地址，例如本机 ComfyUI 使用 http://127.0.0.1:8188。不要把密钥或查询参数写进地址。'), adapterType: adapterType.optional().describe('模型协议适配器。原生 ComfyUI API 图选择 comfyui-prompt。'),
    workflowTemplate: z.string().min(2).max(80000).optional().describe('ComfyUI 导出的 API 图 JSON 字符串，节点含 class_type 和 inputs。界面图先经工作流读取工具自动转换。'), inputMap: z.record(z.string(), z.unknown()).optional().describe('ComfyUI 绑定对象。按 API 图填写 bindings，每项含 role、nodeId、field，视频时长映射还需 durationUnit。不要猜插槽。'), voice: id.optional().describe('供应商音色的 ID 或名称，按配音配置填写。'),
    referenceConfig:referenceConfig.partial().nullable().optional().describe('局部更新参考槽配置。slots 整体替换，null 清除配置。'),
    outputConfig:outputConfig.nullable().optional().describe('局部更新输出配置。画幅尺寸按键合并，null 清除整体配置。'),
    name: z.string().trim().min(1).max(120).optional().describe('Dramlo 显示的配置名称。'),
    provider: z.enum(['custom','openai','deepseek','dashscope','apimart']).optional().describe('供应商类型。自定义供应商可填写 custom 并设置显示名。'),
    providerName: z.string().trim().min(1).max(120).nullable().optional().describe('供应商显示名。null 清除该显示名。')
  }).refine((input) => Object.keys(input).some(key => key !== 'kind' && key !== 'profileId'), '请提供配置修改内容') },
  studio_remove_model: { description: '预览或删除指定模型配置。返回引用、静态可执行性、缺失映射和是否有凭据。当前默认、镜头、扩展或未结束的已提交任务仍引用时会拦截。', schema: z.strictObject({
    kind: kind.describe('配置所属类型。'), profileId: id.describe('studio_list_models 返回的配置 ID。'),
    confirmed: z.literal(true).optional().describe('省略时只预览引用。用户确认后传 true，服务会重新检查引用。')
  }) },
  studio_project: { description: '读取当前项目、制作进度和默认生图、视频配置。传 projectId 校验当前项目，未打开指定项目时报错。镜头覆盖用 studio_project_settings 核对。', schema: z.strictObject({ projectId: id.optional().describe('要核对的项目 ID。省略时读取当前项目；传入时，须先用 studio_open_project 打开该项目。') }) },
  studio_canvas_graph: { description:'读取项目画布节点和连线。计划执行诊断来自实际工作流和输入：ready 可提交，waiting 等上游，blocked 有缺项。返回对应原因和确认用版本。', schema:z.strictObject({projectId:revision.projectId,planId:planId.optional(),nodeId:id.optional().describe('可选画布节点 ID，会附上该节点的上下游和计划执行路径。')}) },
  studio_save_plan: { description:'保存项目的执行计划。可先绑定尚未生成的上游输出，保存不会生成或改写提示词卡。暂停后可编辑未开始的步骤。', schema:z.strictObject({...revision,planId,name:id.describe('在界面向用户显示的计划名称。'),expectedPlanVersion:z.number().int().min(1).optional().describe('更新已有计划时传当前 version，新建时省略。'),steps:z.array(planStep).min(1).max(500).describe('按期望顺序列出步骤；执行器按依赖重排，同级保持给定顺序。更新时保留已启动的步骤和顺序。')}) },
  studio_current_plan: { description:'列出指定项目的未完成执行计划与各步骤状态。用于断线后找回同一计划。', schema:z.strictObject({projectId:revision.projectId,includeCompleted:z.boolean().optional().describe('是否包含已完成的计划，默认只列出未完成计划。')}) },
  studio_run_plan: { description:'用户授权后启动或恢复计划。逐项执行、保存后再推进，失败或结果未知时停下。恢复沿用原任务 ID 并跳过已完成步骤。', schema:z.strictObject({...approvalRevision,planId,expectedPlanVersion:z.number().int().min(1).describe('核对全图时读取的 planVersion。'),expectedExecutionRevision:id.describe('核对全图时读取的 executionRevision，可检测工作流设置与实际输入变化。'),confirmed:z.literal(true).describe('只在用户确认运行或恢复此计划后传 true。'),retryFailed:z.literal(true).optional().describe('用户确认重做当前已知失败的步骤后填 true。仍在运行、可恢复或结果未知时沿用旧任务，不重提。')}) },
  studio_pause_plan: { description:'让执行计划在当前步骤结束后暂停，不取消当前生成。', schema:z.strictObject({projectId:revision.projectId,planId}) },
  studio_project_settings: {
    description: '查看项目目录、画幅及当前镜头实际使用的视频配置，不返回密钥。',
    schema: z.strictObject({ projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), shotId: id.optional().describe('分镜 ID，由 studio_shots 或写入结果返回。请使用 ID，避免把显示镜号误作 ID。') })
  },
  studio_update_project_defaults: {
    description: '修改当前项目画幅、默认视频分辨率或首尾帧链。',
    schema: z.strictObject({
      ...revision, aspectRatio: z.enum(['16:9', '9:16', '1:1', '4:3', '21:9']).optional().describe('项目画幅，宽:高。省略时保留当前画幅。'),
      defaultVideoResolution: z.enum(['768p', '1080p']).optional().describe('项目默认视频分辨率。省略时保留当前值。'),
      h3ShotChainMode: z.enum(['auto','fl2va']).optional().describe('auto 每镜按自己的设置出片；fl2va 只作为未单独设置镜头的默认值，不再强制全部镜头')
    }).refine((input) => input.aspectRatio || input.defaultVideoResolution || input.h3ShotChainMode, '请提供修改内容')
  },
  studio_save_exit: {
    description: '保存活跃项目并请求退出 Dramlo。先确认运行中的镜头已结束或暂停。不会对独立 ComfyUI 发送退出命令。',
    schema: z.strictObject({ ...approvalRevision, confirmed: z.literal(true).describe('用户明确确认当前操作后才填 true。') })
  },
  studio_read_script: { description: '读取剧本标题、关键词、风格、画幅、确认状态及分页正文。传章节 ID 时也返回章节标题。', schema: z.strictObject({
    projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), chapterId: id.optional().describe('章节 ID，可用 studio_project_items 的 chapters 查找。省略时按该工具说明读取整部剧本或不过滤章节。'), offset: page.offset.describe('UTF-16 代码单元偏移，从 0 开始；继续读取用返回的 nextOffset。'), limit: z.number().int().min(1).max(16000).default(8000).describe('本次读取的最大 UTF-16 代码单元数，默认 8000。')
  }) },
  studio_describe_image_inputs: { description: '只读检查服务端生图。ComfyUI 返回固定槽位和候选。OpenAI Images 返回可选参考图张数范围，按提交顺序上传。', schema: z.strictObject({
    profileId: id.optional().describe('图像配置 ID，省略使用当前配置')
  }) },
  studio_set_asset_prompt: { description:'保存镜头或资产卡的生图正文和版式。用 items 批量保存，或用 studio_batch_edit_shots 同时编辑多类内容。', schema:
    editable(assetPromptFields) },
  studio_prepare_image: { description: '保存一项 ComfyUI 或 OpenAI Images 生图预检。不上传、生成或改写项目正文。优先读取本次正文，其次读取项目正文，关键帧缺省时用镜头 visualPrompt。返回 promptSource、版式、画幅、项目参考和预计工作量。', schema: z.strictObject({
    ...videoJob, expectedUpdatedAt: autoRevision, ...generationImageTarget,
    bindingReview, referenceAssignments, aspectRatio:comfyAspect,
    profileId: id.optional().describe('已导入的图像配置 ID。省略时使用当前图像配置'),
    prompt: z.string().trim().min(1).max(12000).optional().describe('实际生图提示词。省略时读取目标已保存的提示词'),
    visualPrompt: z.string().trim().min(1).max(12000).optional().describe('实际送入模型的纯视觉正文。与 prompt 二选一。需要画分格时，用方位直接描述每格画面。'),
    promptLayout: imageLayout,
    referenceAssetIds: z.array(id).max(10).optional().describe('按诊断返回的 ComfyUI 槽位顺序填写项目图片 ID。OpenAI Images 可传 0 至 10 张。canvas-node 省略时读上游图片，传空数组明确不用参考。'),
    outputNodeId: id.optional().describe('仅 ComfyUI。指定 API 图的图片输出节点，省略时使用返回的第一张图片')
  }) },
  studio_confirm_image: { description: '确认并启动已预检生图任务。目标、画幅、参考图和接入仍匹配时，可沿用原计划。旧请求失败时返回 nextAction，修正后用新 ID 预检；结果未知先核对原任务。', schema: z.strictObject({
    ...videoJob, confirmed: z.literal(true).describe('用户确认本次计划后填 true')
  }) },
  studio_resolve_unknown_image: {
    description: '人工核对供应商记录后关闭结果未知的生图请求，保留原记录和错误，不调用模型。只有确认原请求不再运行且没有可恢复结果时使用。',
    schema: z.strictObject({ ...videoJob,
      confirmed: z.literal(true).describe('仅当用户确认已核对原请求、接受用新请求 ID 重新预检时传 true。'),
      reason: z.string().trim().min(1).max(500).describe('简记供应商记录核对结果。勿填写密钥或其他凭据。')
    })
  },
  studio_current_image_job: { description: '读取场景、人物或镜头指定槽位的最近一次生图任务。', schema: z.strictObject({
    projectId: revision.projectId, ...imageTarget
  }) },
  studio_image_status: { description: '按原请求 ID 查询生图状态、失败原因、图片 ID 和是否选用。生成首帧成功时，mainImageSynced 回显是否联动更新主图。', schema: z.strictObject(videoJob) },
  studio_resume_image: { description: '继续等待已有远端 ID 的生图任务。', schema: z.strictObject({
    ...videoJob, confirmed: z.literal(true).describe('确认继续查询原任务后填 true')
  }) },
  studio_attach_image_asset: { description: '把项目已有图片绑定到角色、道具、场景或镜头，已有素材保留。', schema: z.strictObject({
    ...revision, ...imageTarget, assetId: id.describe('项目素材库中的图片 ID')
  }) },
  studio_import_image_asset: { description: '把 8 MB 内的 PNG、JPEG 或 WebP 导入项目素材库。使用固定请求 ID 避免重复导入。', schema: z.strictObject({
    ...videoJob, dataBase64: z.string().max(11184812).describe('图片文件的 Base64，不含 data URL 前缀')
  }) },
  studio_check_video_readiness: { description: '只读检查每镜最终视频提示词、工作流输入和素材。返回首个阻塞原因；不建批次或提交。', schema: z.strictObject({
    projectId: revision.projectId, expectedUpdatedAt: autoRevision.optional(), shots: z.array(videoTake).min(1).max(30).describe('要检查的分镜、工作流配置及素材输入')
  }) },
  studio_reorder_shots: { description: '预览或调整完整的分镜顺序。保留画布位置和剪辑时间线，提示需要复核的连续镜头。', schema: z.strictObject({
    ...previewRevision, shotIds: z.array(id).min(1).describe('按新顺序列出项目的全部分镜 ID，各一次'),
    confirmed: z.literal(true).optional().describe('省略时只预览。用户确认后填 true')
  }) },
  studio_remove_shot: { description: '预览或移除指定分镜及其画布连接和剪辑片段。保留原素材。', schema: z.strictObject({
    ...previewRevision, shotId: id.describe('目标分镜 ID'), confirmed: z.literal(true).optional().describe('省略时只预览。用户确认后填 true')
  }) },
  studio_replace_shots: { description: '一次插入或替换跨场分镜。复用原镜头 ID 保留选片和节点设置。将移除旧镜头时先返回预览，确认后再写入。', schema: z.strictObject({
    ...previewRevision,
    mode: z.enum(['insert','replace']).describe('insert 插入新镜头，replace 替换指定连续范围或整个项目。'),
    afterShotId: id.optional().describe('仅用于插入。填已有镜头 ID，省略时插入开头。'),
    shotIds: z.array(id).min(1).optional().describe('仅用于替换。按原顺序列出连续镜头，省略时替换整个项目。'),
    shots: z.array(shotDraft.extend({
      id: id.describe('复用范围内镜头的原 ID 以保留素材，新增镜头使用新 UUID。'),
      sceneId: id.describe('本镜所属的场景 ID。'),
      chapterId: id.nullable().optional().describe('章节 ID。省略保留原值，null 清除。'),
      voiceover: shotDraft.shape.voiceover.unwrap().optional().describe('对白或旁白，省略保留原值。'),
      audioDescription: shotDraft.shape.audioDescription.unwrap().optional().describe('环境音和动作音，省略保留原值。'),
      subjectIds: shotDraft.shape.subjectIds.unwrap().optional().describe('已有主体卡 ID，省略保留原值。')
    })).min(1).max(60).describe('按新拍摄顺序填写，每镜可选不同场景。'),
    dryRun: z.literal(true).optional().describe('仅返回影响范围与队列状态，不保存。'),
    confirmed: z.literal(true).optional().describe('会移除旧镜头时，用户核对预览后填写 true。原媒体保留。')
  }) },
  studio_crop_shot_frame: { description:'本地裁切已选分格素材中的一格，作为镜头单幅首帧。不会调用模型，保留看板和旧首帧。',schema:z.strictObject({
    ...revision,requestId:videoJob.requestId,shotId:shotSelector,
    sourceAssetId:id.optional().describe('此镜已选看板或标记为分格的关键帧。默认使用已选看板。'),
    panel:z.number().int().min(1).max(36).describe('从左到右、从上到下数第几格，第一格为 1。')
  }) },
  studio_project_overview: { description:'读取全片紧凑制作总览，默认一页 50 镜。返回时长、正文、在盘图/视频和人物绑定。',schema:z.strictObject({
    ...optionalPage,limit:optionalPage.limit.describe('每页最多 50 镜，默认 50。')
  }) },
  studio_shots: {
    description:'分页读取分镜，支持按场、章节或镜号筛选。只查镜号/时长用 summary，用 overview 总览图、视频和人物绑定，默认一页 50 镜。取少量字段用 fields。',
    schema:z.strictObject({ ...optionalPage, limit:optionalPage.limit.describe('每页最多 50 镜；overview 默认 50，其他默认 20。'), detail:z.enum(['summary','overview','full']).optional().describe('summary 返回标题、时长及状态；overview 补在盘图/视频和人物绑定（名字、角色参考图是否在盘）；默认 full。'),
      fields:z.array(id).min(1).max(30).optional().describe('仅返回所选字段，始终附带 id 和 shotNumber。例：[title,duration,plannedStartSec,plannedEndSec]。'),
      shotId:shotSelector.optional(),
      shotIds:z.array(shotSelector).min(1).max(50).optional().describe("按多个镜头 ID 或 #05 格式的全局镜号筛选，与 shotId 二选一。结果按项目顺序。"),
      chapterId:id.optional().describe('由 studio_project_items chapters 查询的章节 ID。'),
      sceneId:id.optional().describe('由 studio_project_items scenes 查询的场景 ID。') })
  },
  studio_append_shots: {
    description: '按给定顺序为已有场景追加结构化分镜草稿和画布节点，不发起生成。为每镜提供新的 UUID，便于在返回结果未知时读取确认。',
    schema: z.strictObject({ ...revision, sceneId: id.describe('场景卡 ID，用 studio_project_items 的 scenes 查询。应对应真实场次。'), chapterId: id.optional().describe('章节 ID，可用 studio_project_items 的 chapters 查找。省略时按该工具说明读取整部剧本或不过滤章节。'), shots: z.array(shotDraft).min(1).max(12).describe('按剧本顺序填写镜头草稿，包含标题、画面、时长、运镜、对白及角色引用。') })
  },
  studio_timeline: { description: '分页读取当前项目的剪辑时间线、字幕、配乐与已有成片。', schema: z.strictObject(optionalPage) },
  studio_export_final: { description: '预检或启动本地基础硬切导出。复用请求 ID 只返回原任务，用 matchesCurrentTimeline 判断是否匹配当前时间线。复杂后期请使用界面导出。', schema: z.strictObject({
    ...previewRevision, requestId: videoJob.requestId.optional().describe('仅预检可省略。确认启动时必须提供 UUID，重复使用同一个 UUID 只返回原任务。'),
    burnSubtitles:z.boolean().optional().describe('省略或 false 时保留项目字幕轨，MP4 不烧录。要求烧录时请用界面导出。'),
    confirmed:z.literal(true).optional().describe('省略时只预检。用户确认时间线后传 true 启动导出。')
  }) },
  studio_export_status: { description: '查询导出状态。省略 requestId 时返回项目最近一次已启动的导出，无记录时 job 为 null。返回相对当前时间线的 matchesCurrentTimeline。', schema:z.strictObject({
    projectId: videoJob.projectId, requestId: videoJob.requestId.optional().describe('导出请求 UUID。省略时按项目查询最近一次已启动的导出。')
  }) },
  studio_batch_edit_shots: {
    description:'一次校验项目版本并按顺序原子保存 1 至 100 项分镜编排、输入、关键帧正文或视频定稿。任一项无效时整批不落盘，无需确认生成。',
    schema:z.strictObject({ ...revision, expectedUpdatedAt: autoRevision, edits:z.array(shotBatchEdit).min(1).max(100).describe('按给定顺序执行的镜头修改列表，支持分别写分镜正文、生图正文及视频定稿。同镜头先改时长再保存视频定稿，警告会按新时长检查。') })
  },
  studio_update_shot_inputs: { description: '修改单镜或用 items 批量更新镜头的时长、视频配置、音频、人物或帧素材。', schema:
    editable(shotInputs) },
  studio_revise_shot: { description: '修改首版支持的分镜文本，保存后返回结果。expectedUpdatedAt 来自项目读取。', schema: z.strictObject({
    projectId: id.describe('Dramlo 项目 ID。用 studio_project 或 studio_list_projects 查询，勿填写项目名称或本地路径。'), shotId: shotSelector, expectedUpdatedAt: autoRevision,
    visualPrompt: z.string().trim().min(1).max(12000).optional().describe('本镜实际可见的画面。写角色站位与朝向、动作、景别、场景锚点、光线及初态到末态的变化。'), voiceover: z.string().max(12000).optional().describe('本镜新对白或旁白。省略时保留，空字符串清除对白。')
  }).refine((value) => value.visualPrompt !== undefined || value.voiceover !== undefined, '至少提供一个要修改的字段') },
  studio_prepare_comfy_node: { description: '预检已启用的 ComfyUI 视频扩展节点。dryRun 只校验并返回实际槽位和输出，不建立计划。保存计划后由 studio_confirm_video 确认。', schema: z.strictObject({
    projectId: revision.projectId, expectedUpdatedAt:autoRevision.optional(), nodeId: id.describe('画布中的 ComfyUI 扩展节点 ID'),
    requestId: videoJob.requestId.optional().describe('保存计划时须填唯一 UUID，dryRun 可省略。'),
    dryRun: z.literal(true).optional().describe('只校验输入、输出与素材，不保存计划或提交模型。'),
    shot: videoTake.describe('分镜和实际输入模式。工作流由扩展绑定。')
  }).refine(input => input.dryRun || (input.requestId !== undefined && input.expectedUpdatedAt !== undefined), '保存计划需要 requestId 与 expectedUpdatedAt') },
  studio_extension_package_preview: { description: '预检 ZIP 扩展包。', schema: z.strictObject({ packageBase64: z.string().max(174764).describe('ZIP 文件的 Base64 内容') }) },
  studio_extension_package_install: { description: '安装已预检的 ZIP 扩展包。', schema: z.strictObject({ packageBase64: z.string().max(174764).describe('已预检 ZIP 的 Base64 内容') }) },
  studio_extension_package_export: { description: '导出扩展版本的 ZIP 包，不含连接配置。', schema: z.strictObject({ typeId: id.describe('扩展 ID'), version: z.number().int().positive().describe('扩展版本') }) },
  studio_describe_extension_tool: { description: '读取项目已启用的第三方 MCP 工具说明和参数结构。', schema: z.strictObject({
    projectId: id.describe('目标项目 ID'), typeId: id.describe('MCP 扩展 ID'), version: z.number().int().positive().describe('已启用的扩展版本')
  }) },
  studio_call_extension_tool: { description: '调用指定项目已启用的 MCP 扩展工具。先用 studio_describe_extension_tool 读取参数。', schema: z.strictObject({
    projectId: id.describe('目标项目 ID'), typeId: id.describe('MCP 扩展 ID'), version: z.number().int().positive().describe('已启用的扩展版本'),
    arguments: z.record(z.string(), z.unknown()).optional().describe('上游工具参数。禁止传入非当前任务需要的敏感资料')
  }) },
  studio_extension_library: { description: '读取扩展库，包括内置、自定义与指定项目的启用状态。', schema: z.strictObject({ projectId: id.optional().describe('可选项目 ID，用于显示项目启用状态') }) },
  studio_extension_preview: { description: '预检本地声明式扩展清单，可声明节点参数、映射和端口。', schema: z.strictObject({ manifest: extensionManifest }) },
  studio_extension_install: { description: '安装自定义节点类型，默认不启用。安装后按项目启用。', schema: z.strictObject({ manifest: extensionManifest }) },
  studio_extension_remove: { description: '卸载指定自定义扩展版本，保留项目节点及素材。', schema: z.strictObject({
    typeId: id.describe('扩展 ID'), version: z.number().int().positive().describe('要卸载的版本'), confirmed: z.literal(true).describe('用户确认卸载后填 true')
  }) },
  studio_extension_enable: { description: '按项目启用或停用指定扩展版本。', schema: z.strictObject({ projectId: id.describe('目标项目 ID'), typeId: id.describe('扩展 ID'), version: z.number().int().positive().describe('要操作的扩展版本'), enabled: z.boolean().describe('true 为启用，false 为停用') }) },
  studio_list_node_types: { description: '列出已注册的画布扩展节点、来源、版本和端口。', schema: z.strictObject({ projectId: id.optional().describe("项目 ID，用于列出该项目启用的扩展") }) },
  studio_add_extension_node: { description: '创建扩展节点，可初始化声明的参数。', schema: z.strictObject({ ...revision, typeId: id.describe('节点类型 ID'), version: z.number().int().positive().optional().describe('扩展版本，默认 1'), nodeId: id.describe('新的画布节点 ID'), videoAssetId: id.optional().describe('提取尾帧使用的项目视频素材 ID'), x: z.number().optional().describe('画布横坐标。'), y: z.number().optional().describe('画布纵坐标。'), params: z.record(z.string(),extensionParamValue).optional().describe('按参数键提供初始覆盖值。') }) },
  studio_describe_extension_node: { description: '读取单个扩展节点的参数定义、默认值、覆盖值、端口和项目修订。', schema: z.strictObject({
    projectId:revision.projectId, nodeId:id.describe('目标扩展实例的画布节点 ID。'), includeChoices:z.boolean().optional().describe('为 true 时向所属 ComfyUI 只读查询模型和枚举选项，null 表示无法获取。')
  }) },
  studio_update_extension_node: { description: '局部更新单个扩展节点的参数，例如 steps、seed 或模型权重。null 恢复默认。保存后重新预检。', schema: z.strictObject({ ...revision, nodeId:id.describe('要编辑的扩展实例节点 ID。'), params:z.record(z.string(),extensionParamValue.nullable()).describe('只提交本次修改的参数。键来自节点定义。') }) },
  studio_connect_extension_node: { description: '按声明的端口类型连接扩展节点。只有一个候选端口时可省略端口 ID。', schema: z.strictObject({ ...revision, sourceId:id.describe('上游节点 ID。'), targetId:id.describe('下游节点 ID。'), sourceHandle:id.optional().describe('上游输出端口 ID。'), targetHandle:id.optional().describe('下游输入端口 ID。') }) },
  studio_run_extension_node: { description: '运行扩展节点。尾帧输出保存到项目素材库，不调用生成模型。', schema: z.strictObject({ projectId: id.describe("当前项目 ID"), expectedUpdatedAt: autoRevision, nodeId: id.describe("画布节点 ID"), arguments: z.record(z.string(), z.unknown()).optional().describe("MCP 扩展的工具参数") }) },
  studio_get_guide: { description: '读取当前 Dramlo 的操作指南。首次使用或更新后先读目录，再按任务读章节。', schema: z.strictObject({ section: z.enum(['startup','models','setup', 'writing', 'production', 'comfyui', 'runtime']).optional().describe('操作指南的章节 ID。省略时返回目录与当前修订。') }) },
  studio_list_skills: { description: '查找 Dramlo 影视短剧制作 Skill，返回适用阶段和触发条件。创作前先找匹配项，再用 studio_read_skill 读取。无匹配项时照常创作。', schema: z.strictObject({}) },
  studio_read_skill: { description: '读取 Dramlo Skill 正文或参考资料，直接用于当前任务。禁止在外部 Agent 的技能目录、配置或工作区安装、复制或注册。', schema: z.strictObject({ skillId: id.describe('studio_list_skills 返回的 Skill ID，按列表中的来源和标识读取。'), referencePath: z.string().min(1).max(160).optional().describe('本机 Skill 返回的 references 中的相对路径。省略时读取 Skill 正文。') }) },
  studio_list_models: { description: '查看指定类型在 Dramlo 中已保存的配置及当前选项。若要找尚未导入的 ComfyUI 工作流，先调用 studio_comfy_discover，再调用 studio_comfy_workflows。', schema: z.strictObject({ kind: kind.describe('接入类型。llm 文本、vision 视觉理解、image 图像、video 视频、tts 配音、music 音乐。') }) },
  studio_describe_model: { description: '只读诊断指定配置，支持非激活项。返回适配器、端点来源、工作流来源、映射及服务端可执行性。ready 是静态检查结果。需要生成时仍须预检具体镜头。不会调用模型或返回凭据。', schema: z.strictObject({
    kind: kind.describe('配置所属类型。'), profileId: id.describe('studio_list_models 返回的配置 ID。')
  }) },
  studio_add_model: { description: '新建模型配置，可选择保留当前默认项。视频的 model 字段填工作流 ID。需要密钥时用 studio_set_credential。', schema: z.strictObject({ select: z.boolean().optional().describe('填 false 时只创建配置，保留当前默认项。省略时设为默认。'),
    kind: kind.describe('接入类型。llm 文本、vision 视觉理解、image 图像、video 视频、tts 配音、music 音乐。'), name: id.describe('用户在界面看到的名称。填写有辨识度的实际名称。'), baseUrl: endpoint.describe('服务根地址，例如本机 ComfyUI 使用 http://127.0.0.1:8188。不要把密钥或查询参数写进地址。'), model: id.describe('供应商的模型标识；对视频接入，按该适配器填写模型或工作流 ID。'),
    adapterType: z.enum(['openai-compatible','openai-images','http-custom','comfyui-workflow','zealman-workflow','comfyui-prompt']).optional().describe('供应商实际使用的适配协议'),
    referenceConfig:referenceConfig.optional().describe('声明原生 ComfyUI 工作流普通图片槽的用途和补位规则。'),
    outputConfig:outputConfig.optional().describe('原生 ComfyUI 可选画幅、目标分辨率和秒数范围。'),
    workflowTemplate:z.string().min(2).max(80000).optional().describe('原生 ComfyUI API 工作流 JSON 字符串。可与声明一次保存。'),
    inputMap:z.record(z.string(),z.unknown()).optional().describe('工作流输入映射。workflowTemplate 已提供时可一起保存。'),
    providerName: z.string().trim().min(1).max(120).optional().describe('自定义供应商的显示名称，独立于协议'),
    provider: z.enum(['custom', 'openai', 'deepseek', 'dashscope', 'apimart']).default('custom').describe('供应商标识，未知或原生本机 ComfyUI 可使用 custom。')
  }) },
  studio_select_model: { description: '按配置 ID 选择已保存的模型。', schema: z.strictObject({ kind: kind.describe('接入类型。llm 文本、vision 视觉理解、image 图像、video 视频、tts 配音、music 音乐。'), profileId: id.describe('已保存的模型接入配置 ID，用 studio_list_models 查询。区别于供应商的模型名。') }) },
  studio_canvas_node_add: { description:'添加自由画布节点，可选连接一个现存的上游节点。', schema:z.strictObject({
    ...revision, type:z.enum(['imageGen','videoGen','audioGen','compose']).describe('自由节点类型。'),
    prompt:z.string().max(8000).optional().describe('节点提示词。'),
    label:z.string().max(120).optional().describe('节点显示名。'),
    position:nodePosition, sourceNodeId:id.optional().describe('需连接的上游节点 ID。'),
    aspectRatio:comfyAspect, duration:z.number().min(1).max(30).optional().describe('视频时长（秒）。')
  }) },
  studio_canvas_node_update: { description:'更新自由节点的正文、标题、位置、时长或产物。', schema:z.strictObject({
    ...revision, nodeId:id.describe('自由节点 ID。'), prompt:z.string().max(8000).optional().describe('节点正文。'),
    label:z.string().max(120).optional().describe('节点显示名。'),position:nodePosition,
    duration:z.number().min(1).max(30).optional().describe('视频时长（秒）。'),
    assetId:z.string().max(160).optional().describe('绑定项目内素材 ID；传空字符串清空目前的选择。')
  }) },
  studio_canvas_node_remove: { description:'移除自由节点及其连线和引用，保留素材原文件。', schema:z.strictObject({
    ...revision,nodeId:id.describe('要删除的自由节点 ID。')
  }) },
  studio_approve_script: { description:'确认或撤回当前项目剧本的制作确认状态。', schema:z.strictObject({
    ...revision,approved:z.boolean().optional().describe('true 确认（默认）；false 撤回。')
  }) },
  studio_pending_jobs: { description:'只读列出未完成的图像预检及视频批次。结果未知须先核对原任务。', schema:z.strictObject({
    projectId:revision.projectId.optional().describe('指定项目 ID，省略时查询当前工作区。'),
    kind:z.enum(['image','video']).optional().describe('只查看图像或视频；省略时两者均显示。')
  }) },
  studio_capture_state: { description:'读取连续拍计划、审批模式、可用产物以及下一场索引。', schema:z.strictObject({
    projectId:revision.projectId.optional().describe('指定项目 ID，或省略使用当前打开的项目。')
  }) },
  studio_capture_set_plan: { description:'写入连续拍计划并建立画布。默认整批一次确认，支持逐场。只保存，不提交生成。', schema:z.strictObject({
    ...revision,plan:capturePlan,approvalMode:captureApprovalMode,
    source:z.string().max(150000).optional().describe('本计划所依据的剧本正文；与当前剧本相同时，之后改剧本会使旧计划待复核。')
  }) },
  studio_capture_prepare: { description:'从当前连续拍画布准备可复用的服务端执行计划，不提交生成。用 studio_canvas_graph 预览，再用 studio_run_plan 确认、启动或恢复。', schema:z.strictObject({
    ...revision,captureId,sceneIndex:captureSceneIndex
  }) },
  studio_capture_approve: { description:'标记手动制作的旧连续拍计划阶段已完成。先核对必需产物是否真实归档；不启动生成。新计划用 studio_capture_prepare 和 studio_run_plan。', schema:z.strictObject({
    ...revision,captureId,phase:z.enum(['assets','videos']).describe('标记本场素材已齐或整批视频已齐。'),sceneIndex:captureSceneIndex
  }) },
  studio_extract_tail_frame: { description:'截取真实视频的最后一帧并归档到项目素材库。不会调用模型。', schema:z.strictObject({
    ...revision,videoAssetId:id.optional().describe('项目视频素材 ID；与 nodeId 二选一。'),
    nodeId:id.optional().describe('已经有视频产物的画布节点 ID；与 videoAssetId 二选一。')
  }).refine(value => Boolean(value.videoAssetId) !== Boolean(value.nodeId),'指定素材 ID 或节点 ID，两者二选一。') },
}
