# Dramlo 操作手册

本文记录 Dramlo 的现有操作。标为「待实现」的步骤按[产品基线](studio-product-goal.md)验收。本机原生 ComfyUI 制作可使用 Web 工作台与 Rust 服务。真实 H3 生成属于实机验收。

## 1. 启动与数据

- 首页、项目管理、项目画布、素材库和设置共用左侧导航及顶栏。画布快捷工具和节点检查器在项目区右侧。设置分类显示在固定导航旁。
- 运行 Windows 构建包内的 `start.cmd`，或使用配置好的桌面快捷方式。
- 在「设置」确认保存目录。Web 设置单独保存在所选工作区的 .dramlo 目录。切换工作区时按各自的配置运行。项目和素材按 Dramlo 所选保存目录管理。
- 外部 Agent 开始时用 `studio_project` 读取项目 ID、名称和更新时间。已知项目 ID 时传入 `projectId`，与当前项目不一致会报错。用 `studio_list_projects` 查找项目，用 `studio_open_project` 切换后重读。创建新项目用 `studio_create_project`。
- 从界面或 MCP 创建本机项目时，项目根目录会生成 `AGENTS.md`。接手时先读 `studio_project` 返回的 `projectInstructionsPath` 所指文件。有值得复用的发现时更新项目已有经验文档，首次没有指定位置时使用 `docs/创作经验.md`。
- 移除项目时用 `studio_delete_project` 预览。切离目标项目并核对未结任务后，取得用户确认，再传预览的 `updatedAt` 和 `confirmed: true`。项目会从列表移除，文件保存在返回的 `recoveryPath`。如需恢复，可在未打开该项目时把目录移回 `originalPath`。
- 通过 Agent 启动或复用应用、保存并退出、遇到活动任务时告知影响，属于待实现的应用管理接口。当前可用界面启动和退出。

在「设置 → 系统」导出项目 ZIP。ZIP 包含项目正文、素材清单、媒体、项目文档和项目根目录的 `AGENTS.md`。供应商配置、密钥、公共素材库、独立剧本库和运行中的任务记录不随包迁移。新工作区导入时按项目 ID 重新建立目录，有同 ID 项目会停止。ZIP 导入要求项目文件使用当前格式。原始项目目录可打包留存，媒体可按需导入新项目。

网页的项目移除也使用可恢复目录。项目素材、剧本和公共素材移除时会先检查清单，项目素材仍被引用时应先解绑。网页项目管理会自动更新列表。外部 Agent 用 `studio_remove_project_asset` 预览媒体或角色卡移除，核对引用并获用户确认后，带上预览的 `updatedAt` 和 `confirmed: true`。角色卡移除保留角色板。媒体移除将文件移入可恢复目录。

## 2. 模型、供应商和工作流

设置默认显示外部 Agent、ComfyUI、图像、视频、配音和系统。语言模型、视觉理解、AutoDL 和 Zealman 可在「可选模型与供应商」中展开。使用外部 Agent 写作和审校时，可以跳过 Dramlo 的语言和视觉模型设置。

外部 Agent 使用自己的模型和会话创作，通过 MCP 写入剧本、分镜和提示词。生图、生视频使用 Dramlo 项目实际选用的工作流或供应商。本机 Web 的剧本、分镜与提示词由用户手动编辑，或交给外部 Agent。

1. 在「设置」中选择图像或视频的接入，填写 ComfyUI 本机地址。本机主实例为 `http://127.0.0.1:8188`。配置模型 ID、工作流 ID 或原生 ComfyUI API 图，按对应接入类型填写。
2. 常规接入在「设置 → ComfyUI」扫描本机、浏览已保存的工作流并直接导入，或让 Agent 通过同一 MCP 连接执行。需要自带工作流文件时，也可以在 ComfyUI 中导出 API 格式。Dramlo 接收的 API 图包含带 `class_type` 和 `inputs` 的节点。在「设置 → 图像生成」或视频接入设置中，选择对应接入，在「Comfy 工作流模板」区域从文件导入或粘贴后校验保存。检查提示词、普通参考图、首帧、尾帧和输出节点的绑定。在线供应商的密钥在设置页管理。
3. MCP 用 `studio_list_models` 查询配置 ID，用 `studio_describe_model` 静态检查配置。未激活的配置也可查询。端点只返回协议、主机和端口。Web 中检查本机 ComfyUI 是否在线，打开「设置 → ComfyUI 接入」，点「扫描本机」查看状态。在线供应商目前没有通用的连接测试入口。按镜头预检通过后，如需确认实际生成能力，再经用户确认试跑。配置写入用 `studio_add_model`、`studio_select_model` 和 `studio_model_update`。
4. 区分配置保存、接口响应、实际生成三项结果。核实工作流节点、权重选项、输入素材和绑定后，才发起单镜实跑。Dramlo 的同一个 MCP 入口支持 ComfyUI 发现、读取和导入。

模型改名、修改供应商标识或显示名可用 `studio_model_update`，也可修改未激活项。用 `studio_remove_model` 预览删除影响、静态可执行性和缺失映射，再传 `confirmed: true`。当前默认配置、镜头或节点引用、扩展清单引用及未结束的已提交任务会阻止删除。仅预检的计划会列出，删除后须重新选择配置并预检。

用 `studio_set_credential` 写入用户授权的 API 密钥，传入配置 ID 和经核对的完整服务地址。响应只返回 `hasCredential`。写入仍沿用应用现有的模型设置存储。若 Agent 客户端会记录工具参数，可在设置页填写以避免密钥进入会话记录。

用 `studio_describe_model` 读取 `workflowRevision`，再用 `studio_patch_workflow` 修改现有节点字段的标量值。映射和连线会保留。返回的 `mappedRole` 表示该字段可能在生成时被项目输入覆盖。

`studio_describe_model` 和 `studio_remove_model` 都在顶层返回 `mappingChecked` 和 `missingMapping`。值为 `true` 和 `[]` 表示已检查且没有缺项，`false` 和 `null` 表示无法检查或不适用。配置是否可执行仍以 `ready` 为准。模型诊断保留旧的 `workflow.missingMapping` 字段。

## 3. 接入外部 Agent

首次在「设置 → 外部 Agent 接入」复制接入说明，配置一次 MCP。之后继续使用同一个连接入口。stdio 客户端用 `studio_web_status` 检查服务，直接调 HTTP 桥接时用带鉴权的 `GET /api/dramlo/mcp-capabilities`。两种方式都用 `studio_get_guide` 获取指南目录和修订，按任务读取章节。章节号沿用完整手册。

设置页复制的 `mcp studio-compact` 配置默认提供五个工具。用 `studio_find` 搜任务关键词或工具名，用 `studio_call` 将工具名和参数对象传给现有操作。本章其余部分使用操作原名，例如读取指南时调用 `studio_call`，传 `name: "studio_get_guide"` 和 `arguments: {"section":"writing"}`。服务未启动时，先查找并调用 `studio_start_web`，随后再读取项目。已有脚本可继续使用 `mcp studio` 获取完整工具清单。操作顺序和确认边界统一查本手册。

创作时按当前阶段用 `studio_list_skills` 找编剧、分镜、资产、提示词或验收手册，用 `studio_read_skill` 读取正文和所需参考。H3 视频按实际输入模式选读 `h3-prompt-writing` 的三段或六段格式。按 `revision` 判断何时重读。直接用于本次任务，禁止安装、复制或注册到外部 Agent 环境。没有匹配项时照常继续。

Dramlo 自带扩展库。用 `studio_extension_library` 查看指定项目的扩展。第三方 MCP 工具先通过 `studio_describe_extension_tool` 读取实际参数，再用 `studio_call_extension_tool` 调用。工具执行器由用户在扩展库授权，外部 Agent 沿用原 Dramlo 连接。详见[扩展库与导入规范](studio-extension-library.md)。

### 按任务选工具

- 开始项目时用 `studio_project` 读取项目和默认图像、视频配置。需要解释镜头覆盖时用 `studio_project_settings`。
- 查看镜头顺序、时间和状态时，用 `studio_shots` 的 `detail: "summary"`。读写提示词前再用完整条目。
- 创作时，按 `studio_list_skills` 的 `stage` 和 `trigger` 选择规则。H3 快速写法与完整参考各有入口。
- 排查失败时读取 MCP 的 `structuredContent.code`。冲突 `conflict` 重读项目并沿用最新修订，`invalid` 修正输入，`not_found` 核对 ID，`busy` 查询队列。写入成功时可沿用其返回的 `updatedAt`。
- 先用 `studio_check_video_readiness` 检查；`studio_prepare_video` 会保存待确认计划。生图先用 `studio_describe_image_inputs` 核对参考槽，再准备计划。
- 安装、启停扩展和管理 ComfyUI 实例时查阅对应章节。涉及执行程序和生成的操作按界面或工具要求确认。

### Web 版首次接入

「设置 → 外部 Agent 接入」提供一份连接说明。连接后按需读取在线指南和创作 Skill。

Dramlo 工作台、创作 Skill、ComfyUI 发现与导入统一通过 `mcp studio-compact` 使用。连接后只读发现本机 ComfyUI，未发现也可以继续使用其他 Dramlo 工具。

1. 运行构建包内的 `start.cmd`，打开终端打印的地址。默认端口为 `38042`，工作区为构建包内的 `projects` 目录。自行构建时按[本机 Web 工作区](#本机-web-工作区)启动。
2. 在「设置 → 外部 Agent 接入」点「复制连接说明给 Agent」或「下载接入说明」，也可以在外部客户端手动添加 stdio MCP。命令填发行包内 `dramlo-server.exe` 的绝对路径，参数按数组逐项填写。以下是常见的 JSON 配置示例。桌面启动入口在原目录应用更新，现有 MCP 配置可沿用。手动迁移安装目录时更新程序路径，按需替换 ComfyUI 授权目录。

~~~json
{
  "mcpServers": {
    "dramlo": {
      "command": "D:/Dramlo/dramlo-server.exe",
      "args": ["mcp", "studio-compact", "--token-from-launcher"],
      "env": {
        "DRAMLO_WEB_MCP_URL": "http://127.0.0.1:38042",
        "DRAMLO_WEB_WORKSPACE": "D:/DramloWorkspace"
      }
    }
  }
}
~~~

3. 桌面启动版使用 `--token-from-launcher`。同一 Windows 用户会自动使用程序旁的 `launcher-token.dpapi`，客户端配置无需填令牌。若桌面服务已配置 FFmpeg，复制出来的 MCP 配置也会带上该路径，供冷启动后截取尾帧。指定工作区后，`studio_web_status` 可以在离线时返回状态，`studio_start_web` 可在首次启动时创建令牌并启动工作区。手动部署且已有自己的令牌时，省略此参数，由客户端传入与服务一致的 `DRAMLO_MCP_BRIDGE_TOKEN`。

客户端用 MCP 的工具列表获取每个输入字段的填写提示。工具说明标注 ID 来源、示例、单位、默认值和确认条件。外部 Agent 勿把界面占位符原样写进剧本或分镜。

4. 调用 `studio_list_projects` 和 `studio_project` 核对连接和活动项目。活动项目为空时，可以在网页打开项目，也可调用 `studio_open_project`。通过同一连接调用 `studio_comfy_discover` 发现本机实例。用发现结果中的 `target` 查看工作流和槽位，再用同一目标导入。长期管理或托管启停时才保存实例。这些检查不会发起生成。
5. 无法连接时，确认服务地址、客户端是否加载了 MCP、程序路径和令牌是否一致。ComfyUI 列表为空时，检查授权目录和服务地址。需要项目自带的创作规则时，可用 `studio_list_skills` 和 `studio_read_skill` 按需读取。

### 本机 HTTP 桥接调试

常规客户端通过 stdio 接入。调试桥接时，可向 `POST /api/dramlo/mcp` 发送以下请求。`input` 必须有，工具无入参时用空对象。

~~~http
POST /api/dramlo/mcp HTTP/1.1
Host: 127.0.0.1:38042
Authorization: Bearer <与服务一致的令牌>
Content-Type: application/json; charset=utf-8

{"name":"studio_project","input":{}}
~~~

只从本机回环地址调用。能力接口的 `tools` 列出 HTTP 可调用工具，`stdioOnlyTools` 列出仅 stdio 客户端提供的工具。失败返回 HTTP 状态和错误 JSON，例如鉴权失败为 403，项目修订冲突为 409。JSON 响应使用 UTF-8。终端排查 stdio 时按 UTF-8 读取子进程原始 stdout，每行一个 JSON-RPC 消息。

普通项目写入和生成预检可传 `expectedUpdatedAt: "auto"`，服务端使用本次读取的版本。按镜号修改时使用当前顺序。需按已核对版本确认的操作继续传数字，包括移除项目、确认分镜重排或移除、启动执行计划、确认导出及保存退出。项目写入遇到修订冲突时，比较响应里的 `currentUpdatedAt` 和 `expectedUpdatedAt`。重读项目并核对期间的修改，再决定是否重试。其他类型的 409 可能没有这两个字段。

「创作 Skill 库」会列出本机发行包随附的创作资料。外部 Agent 用 studio_list_skills 查看目录，再用 studio_read_skill 读取正文或返回的 referencePath。随附的第三方 Markdown 保留来源，纳入本机包。

## 4. 剧本、分镜与画布

1. 在 Dramlo 创建或打开项目，按场整理剧本。在界面确认原文与制作范围。
2. Agent 先按已选范围读取 `studio_read_script`，再用 `studio_shots`、`studio_project_items` 查看分镜、人物、场景、道具、节点和连线。需要已导入的导演手册时，从 Dramlo Skill 列表选择并读取正文。
3. 新项目依次调用 `studio_import_script` 保存剧本原文和章节，`studio_upsert_project_assets` 创建人物、场景和道具卡片，再用 `studio_append_shots` 把分镜草稿追加到已有场景。传入最新的 `expectedUpdatedAt`、场景 ID、可选章节 ID、人物及道具 ID，每镜生成一个 UUID。Dramlo 会把镜头、画布节点和连线保存到当前项目。用 `studio_shots` 按章节或场景核对结果。
4. 用 `studio_update_script` 局部更新剧本。只改标题、关键词或风格时可省略正文，保留剧本确认。正文实际变更才重置确认。用 `studio_read_script` 读回元数据和正文，用 `studio_rename_project` 改项目显示名称，用 `studio_revise_shot` 修改指定镜头。写入前提供项目修订号。
5. 用 `studio_canvas_graph` 查节点和连线。自由节点用 `studio_canvas_node_add`、`studio_canvas_node_update`、`studio_canvas_node_remove` 增删改；人物、场景、道具和镜头从各自的数据工具修改，画布会按数据重建。画布位置独立于分镜顺序。用户改提示词或参考图后，按新状态准备提交；遇到版本冲突，重读并保留人工修改。

Agent 用 `studio_shots` 读取 `plannedStartSec` 和 `plannedEndSec`。时间按全项目镜头顺序和时长计算，筛选场次时沿用整片时间。标题只写镜头名称。用 `studio_reorder_shots` 调整全项目顺序，或用 `studio_remove_shot` 移除指定镜头。先预览受影响的画布、剪辑片段和接续镜头，再提交用户确认。移除保留源素材。项目有未结制作批次时先处理批次。

### 连续拍

连续拍按场次和子镜头建画布。每场从独立开场图起拍，同场后续镜头使用上一镜实际归档并选用的尾帧。使用支持单首帧的原生 ComfyUI 工作流。场景开场图和子镜头的普通参考图必须符合各自工作流的输入能力。

1. 在画布顶部打开「连续拍」，导入结构化 JSON，或由 Agent 调用 `studio_capture_set_plan`。默认整批确认一次，选择逐场确认时每场单独预览并确认。保存只写计划和画布。
2. 用「预览执行计划」查看依赖、实际配置和等待项。Agent 调 `studio_capture_state` 获取计划 ID，逐场模式也读取待处理场次，再调 `studio_capture_prepare` 和 `studio_canvas_graph`。重新准备同一批或同一场会找到已有计划。
3. 用户确认后，在面板启动，或调用 `studio_run_plan`。服务端串行运行，后一步等待上游归档。用 `studio_current_plan` 查进度，用 `studio_pause_plan` 暂停后续步骤。恢复时重新确认当前输入，已完成的步骤继续保留。
4. 改提示词时编辑画布节点。更换整个计划会移除旧计划对应的画布节点，保留素材原文件。存在运行中或待核对任务时先处理原任务。

单独截帧时，在画布选中有视频产物的节点后点「截取所选视频尾帧」，或用 `studio_extract_tail_frame` 传节点 ID 或视频素材 ID。尾帧保存到项目素材库。旧版手动连续拍工具 `studio_capture_approve` 用于标记已经归档的阶段，资产阶段须传同一场次索引，重试不会推进到下一场。

### 画布从哪里读取

剧本、分镜、场景和素材绑定驱动固定制作节点。同步画布会从这些数据重建固定节点和连线。优先改实际来源。例如从镜头修改人物，从场景卡换图，再刷新画布。自由生图、自由视频、合成和扩展节点是独立保存的画布内容。上游节点作为参考来源时保留节点引用，下游随上游当前输出更新。

### 画布写操作

把人物、场景或道具节点的绑定端口拖到镜头节点左侧的端口。双击绑定线可以断开，也可以在镜头属性的「镜头绑定」里编辑。人物首次手动编辑时按当前有效列表切到显式绑定。更换场景会替换当前场景，清空后保持未分配。道具可以清空，也可以恢复按文案推断。要让外部 Agent 操作当前画布对象，先选中并点击「复制选区给 Agent」，再把对象信息和任务一起交给 Agent。

Agent 用 `studio_update_shot_inputs` 或 `studio_batch_edit_shots` 的 `inputs` 项保存绑定。省略字段表示保留，`sceneId: null` 表示明确未分配，`propIds: []` 表示无道具，`propIds: null` 恢复道具推断。人物清空时传 `subjectBindingMode: "explicit"` 与 `subjectIds: []`。保存后用 `studio_canvas_graph` 或 `studio_shots` 读回。

Agent 用 `studio_batch_edit_shots` 的 `shot-fields` 项编辑镜头标题、景别、站位、视线、连续性、道具焦点、表演稿及分镜审阅字段。剪点和观众新知在分镜页读回，标题同步到已打开的画布。服务端视频批次运行时，画布节点显示对应镜头的制作状态。

自由节点用 `studio_canvas_node_add` 创建，`studio_canvas_node_update` 改正文、显示名、坐标或视频时长，`studio_canvas_node_remove` 删除并清掉相连的连线。三种工具只接受 `imageGen`、`videoGen`、`audioGen`、`compose`。

流水线节点不通过这些工具增删。人物、场景、道具用 `studio_upsert_project_assets` 改，镜头用 `studio_append_shots`、`studio_revise_shot`、`studio_update_shot_inputs` 改，固定节点和连线会按数据重建。对流水线节点调用删除会被拒绝。

坐标可省略，服务端放在最右一列、一列满四个再换列。要接在某个节点后面时传 `sourceNodeId`，会自动连线，同一对节点只连一次。合成节点全项目只有一个，重复创建会返回已有节点。

自由节点的图走项目里配置好的图像模型，和给镜头出图是同一条链路：`studio_prepare_image` 传 `targetType: "canvas-node"`、`targetId` 填节点 ID。提示词读节点正文，画幅沿用项目设置，结果写进节点产物并记入历史版本。

没传 `referenceAssetIds` 时，服务端自动取上游节点的产物作参考，最多三张；上游还没生成就按纯文生图处理，返回的 `warnings` 会写明取了几张。

如果你的模型比平台配置的更强，也可以自己生成后走次路：`studio_import_image_asset` 导入素材库，再把 `assetId` 传给 `studio_canvas_node_update` 挂到节点上。传空字符串清除。

### 确认边界

默认可直接完成用户要求的操作，不必逐步请示。以下三类先确认：

- 会批量消耗额度：整片重生视频、重新提取全部人物与场景、全量重出新看板
- 不可逆的批量删除：删除项目、移除全部素材、清空分镜
- 一次改动超过二十镜或整集重写

改文本不等于同意付费生成。用户说「改提示词」「换个说法」时只改字，不触发重生。

分镜未确认前不生成看板、视频和导出。用户说「确认」「可以」「继续」才算确认，用 `studio_approve_script` 记录；正文改动会自动重置为未确认。没有正文时无法确认。

### 职责分工

外部 Agent 负责创作和画布：写剧本、拆镜、写资产与镜头提示词、安排节点与连线、检查缺项。

具体的图、视频、音频一律用项目里配置的模型生成，走 `studio_prepare_image`、`studio_prepare_video` 这类预检入口，不要绕过平台自己出图再导入。只有用户明确要求「用你自己的模型」时，才走 `studio_import_image_asset` 加 `studio_canvas_node_update` 的次路。

模型选择权在用户手里：默认用项目当前配置，用户点名某个配置时传 `profileId`，不要自行切换默认项。

## 5. 生成、暂停和找回素材

编辑时间线允许空隙和重叠。保存返回非阻断 `warnings`，保存和读取都返回全时间线的 `timelineIssues`，即使只读一页片段。基础导出遇到这些问题会拒绝，错误的 `issues` 列出相邻片段 ID 和相差秒数。成片过期由 `finalVideoStale` 独立表示。

导出预检可省略请求 ID。确认启动须提供 UUID，同一个请求 ID 只执行一次。`studio_export_status` 只传项目 ID 时返回最近一次已启动的导出，无记录时 `job` 为 `null`。用 `matchesCurrentTimeline` 判断原任务是否对应当前时间线。为 `false` 时，若要导出新版时间线，使用新的请求 ID。为 `null` 时，旧记录没有足够信息做比较。

用 `studio_edit_timeline` 从已选素材组装或保存时间线，传入读取时的项目版本。用 `studio_timeline` 读回片段、成片 ID 和 `finalVideoStale`，省略分页参数时读取前 20 条。该字段为 `true` 表示成片对应旧的导出输入，为 `null` 表示无成片或无可比对记录。旧成片可继续预览。基础硬切用 `studio_export_final` 预检，用户确认后用同一请求 ID 启动，再用 `studio_export_status` 查询结果。字幕轨会保留在项目中，默认不烧录到 MP4。片段自带的声音随片段保留。独立配乐、转场、变速、独立配音、叠字或需烧录字幕时使用界面导出。

### 未结任务

预检只保存计划，不生成，也不花钱。计划未确认时一直挂着，用 `studio_pending_jobs` 查看，按状态处理：

- `prepared`：还没提交，可以直接撤销。同一目标重新预检时，旧的会自动作废。
- `submitting`、`waiting`：正在跑，先查状态，不要撤销。
- `unknown`：请求已发出但没拿到结果，可能已产生费用或已生成。必须核对供应商后台，再用 `studio_resolve_unknown_image` 关闭。这类任务会阻止同一目标的新预检。

超过一天的 `prepared` 在查询里标 `stale`，但不会自动作废——预检不花钱，自动清理可能误删还在准备中的计划。

### 持久化执行计划

先保存剧本、分镜和资产卡。用 `studio_save_plan` 保存生图、逐镜视频、提取尾帧和基础导出步骤。生图默认读项目正文，需要本次用词时在该步骤单独指定。镜头视频读取已保存的最终提示词。素材尚未生成时也能先连依赖，例如把提取尾帧步骤接到下一镜首帧。工具说明列出 `sources`、`after` 的参数及导出如何引用归档版本。

用 `studio_canvas_graph` 指定项目和计划，核对画布连线、执行依赖、工作流和必要输入。配置或槽位错误会显示 `blocked`。已明确连接上游且仅缺产物时显示 `waiting`。`ready` 表示此步当前可以提交。导出等待项会注明上游步骤及对应片段；缺素材且未绑定上游的片段会在 `reason.issues` 列出片段和镜头 ID。`buildFromShots: true` 会纳入全部分镜，只导出部分镜头时用 `clips` 指定。画布连线和执行依赖分别在 `edges` 和 `executionEdges` 中，执行判断以 `execution` 为准。

搭建和预检阶段不启动生成。向用户展示制作范围、实际工作流和执行依赖，处理阻塞项后，由用户在画布的「执行计划」中启动，或明确授权 Agent 调 `studio_run_plan`。一次确认启动整份已核对的计划。用 `studio_current_plan` 查询进度，用 `studio_pause_plan` 在当前步骤归档后暂停。重连先找原计划，恢复时跳过已归档步骤。结果未知时保留请求 ID，先核对原任务；确认失败且用户同意重做时才重试。任意外部 MCP 插件和复杂后期仍使用各自入口。

未提交的预检计划会跨重启保存。用原项目 ID 和请求 ID 调 `studio_discard_video_plan` 或 `studio_discard_image_plan` 可以撤销。撤销不调用模型，也不删除素材。原请求 ID 仍可查到 `discarded`，重做须用新请求 ID。

### 工作流图片输入

首帧、尾帧和普通参考图在预检时按 API 图连线区分。混合工作流可同时提供首帧与 `referenceAssetIds`，参考图按槽位顺序填写。H3 I2VA 的第一个参考槽可作为首帧。工作流仍引用未绑定的固定图时，图像和视频预检会拦截并列出文件名。补齐项目参考图或从工作流明确移除相应输入后再预检。视频的 `allowTemplateImages: true` 会保留这些文件并实际用于生成，须由用户确认具体文件。指定首尾帧模式前，核对工作流确实有尾帧槽。

生图映射未通过时，用 `studio_describe_image_inputs` 读取失败原因、现有槽位和正向提示词候选。用 `studio_model_update` 更新映射时，可保留已保存的 API 图。道具多角度图可在道具节点运行；Agent 可用服务端生图选择 `targetType: "prop"`。

生图预检会提示可能被画进画面的画幅标签、设定表标题和分格补齐指令。提示不会自动改稿。画幅诊断对照项目比例和工作流里可识别的 `ResolutionSelector` 设置，确认前按需调整。无法识别时 `aspect.matchesProject` 为 `null`。最终像素尺寸以实际生成结果为准。

用 `studio_set_asset_prompt` 把生图正文和版式保存到对应镜头或资产卡。标题、预期画幅和行列数放在 `promptLayout`。需要分格时，在 `visualPrompt` 中写每格可见内容。预检优先用本次提供的正文，其次读项目保存的生图正文。关键帧缺少专用正文时，用镜头的 `visualPrompt`。查看 `promptSource` 核对取词来源。只提交视觉正文，版式资料单独保留。预期画幅仍须核对实际工作流设置。视频最终提示词和镜头设置保存后，检查 `warnings` 中的镜内节拍与时长提示。

H3 Image 的 `StringReplace` 占位符可从连线识别。只有一个明确的参考图入口时可自动识别，多候选需确认顺序。有人物绑定但工作流不接收参考图时，生图计划会提醒。模型列表的 `serverExecutionSupported` 表示接入类型是否可由本机服务执行，工作流仍需预检。创建模型时传 `select: false` 可保留现有默认配置，也可直接按 ID 修改非默认项。

### 预检怎么看

在本机 Web 工作台用 ComfyUI 生成普通镜头视频时，画布节点、镜头属性面板和视频页的单镜入口与原生批量入口共用服务端预检。点生成后，在视频状态栏展开待确认计划，核对终稿、画幅、时长和实际素材槽。确认前可以区分普通参考图、首尾帧、上一镜尾帧、补位图和工作流固定图。外部 Agent 在预检和批次状态的 `mediaInputs` 里读取项目素材与尾帧来源，工作流固定图见 `templateImages`。预检不会改存镜头终稿或调用 ComfyUI 开始生成。

使用节点属性里的服务端生图，或在配图页用 ComfyUI API 工作流、OpenAI Images 配置预检单镜时，可以先填入镜头参考图，再核对提示词来源、实际参考图顺序、对应槽和补位状态。配图页批量入口和其他图像供应商沿用原有生成流程。

预检的 referenceFacts 列出项目的人物和场景、当前选用的参考图，以及实际提交的素材 ID。用这些引用约束外貌和材质，只在正文写本镜可见的动作或明确的状态变化。项目记录与本次提交可能不同，两者都应核对。

workload 给出预计提交次数、图片输入数、请求尺寸及视频秒数，画布计划总览合计尚未完成的生成步骤。moneyStatus 为 not_estimated 时，金额与 GPU 用时没有估算依据。费用、设备耗时和实际生成仍以供应商及本机运行记录为准。

照片审美或光线差异交给看图复核。确定的输入错误（已标记看板进入首尾帧、正文提到未绑定人物、实际工作流仍引用未确认的固定图）在提交前拦截。已明确绑定且有角色板的人物，即使正文只写“他”或“她”，参考型生成也会核对本次有没有带正确的参考图。

### 服务端生图

预检核对正文里可识别的人物名称（或已记录的提示词名称）、分镜绑定和本次参考图。遇到 400 时读取 issues，按人物与素材归属修正。画外提及、替用参考图或有意跳过参考时，可在本次输入用 bindingReview 说明已核对原因。工作流没有参考槽时，预检仍会给出无参考图的提醒。视频首尾帧预检会拒绝已标记的分格看板。

OpenAI Images 配置也可由 MCP 和执行计划使用。用 `studio_describe_image_inputs` 检查配置，再把项目内的图片 ID 按参考顺序放进 `referenceAssetIds`。可传 0 至 10 张，支持 PNG、JPEG 和 WebP。空列表走文生图，有参考图走图片编辑。预检显示送出的画幅尺寸，用户确认后才向配置的供应商提交。结果未知时先核对供应商记录，避免重复计费。

先用图像端的原生 ComfyUI API 工作流配置正向提示词和需要的参考图槽位。用 `studio_prepare_image` 提交场景或镜头目标、项目修订、实际提示词和按槽排序的项目参考图 ID。要跟随项目画幅，为已保存配置增加 `aspect_ratio` 映射。新导入的标准 ResolutionSelector 会自动建此映射。未映射的旧工作流沿用模板设置。核对计划后用同一个请求 ID 调用 `studio_confirm_image`，用 `studio_image_status` 查进度和图片 ID，或用 `studio_current_image_job` 查该目标最近的任务。若已知远端任务 ID 且等待中断，用 `studio_resume_image` 查询原任务。结果未知时先核对原任务。确认原请求已停止且没有可取回结果后，用户可用 `studio_resolve_unknown_image` 附核对说明关闭原记录。旧请求仍可查询，重新生成用新请求 ID。计划内步骤可在关闭原记录后重新确认运行并传 `retryFailed: true`。已明确失败时，状态会给出 `nextAction`。修正原因后用新请求 ID 预检，复用旧 ID 只读回旧结果。OpenAI Images 的状态返回 `providerError`，含可识别的审核、参数、权限或额度原因及供应商请求 ID。预检后若只有其他项目字段变化，仍可确认。目标镜头、项目画幅、参考图或接入配置变化时，重新预检。

生成图片进入项目素材库。归档时只有目标、画幅、参考图和工作流接入仍匹配才自动选用。生成途中改了相关输入时，图片仍归档，可手动选用。场景和镜头配图节点的检查器也提供同一服务的预检、确认和状态入口。其他图像供应商仍沿用现有界面路径。

用 `studio_import_image_asset` 导入自制图片，再用 `studio_attach_image_asset` 绑定到项目场景、角色、道具或镜头。素材清单可用 `studio_project_items` 的 assets 查询。


用 `studio_check_video_readiness` 逐镜检查最终提示词和工作流输入。检查采用正式视频预检的校验逻辑，不建立批次。出片读取 `compiledVideoPrompt`。原生视频队列把定稿直接写入工作流的提示词槽。需要改写时在定稿前完成，工作流内部另有处理节点时以该工作流为准。追加草稿时可直接提供已定稿的值，也可之后用 `studio_set_video_prompt` 保存。用 `studio_describe_model` 检查 `hasFirstFrame`、`hasLastFrame` 和参考槽。仅有首帧入口时，用上一镜尾帧续拍仍是本镜的首帧生成。

镜头主图用于展示和缺少视频时的静帧剪辑。首帧模式读取首帧槽或本次显式输入。生成新首帧时，主图为空或仍指向同一镜头的旧首帧才自动更新。手动绑定 `image` 或 `first` 分别更新各自槽位。用 `studio_shots` 的 `imageConsistency` 读回同图（`same`）、不同图（`diverged`）、仅主图（`image_only`）、仅首帧（`first_only`）、双空（`empty`）。生图状态用 `mainImageSynced` 核对当次是否联动。

镜头检查器显示配图策略、人声模式和缺图道具。用 `studio_project_settings` 传镜头 ID 读取对应设置，并用 `studio_update_shot_inputs` 修改逐镜视频配置、时长、音频模式、人物绑定、画布首尾帧和帧素材。选择某镜的视频配置不会切换全局默认，传 `videoProfileId: null` 可恢复跟随默认。视频预检依次使用本次计划指定的配置、镜头指定的配置或全局默认。校验通过后返回实际配置 ID、输入模式和帧来源。仅显式绑定时，空人物列表表示本镜无人。没有指定人声模式时，绑定独立音频会记为配音。未绑定音频时，有台词按出镜对白处理，其余按无口播。

用 `studio_shots` 的 `summary` 快读镜号和时长，用 `studio_project_overview` 看全片紧凑状态，或用 `studio_shots` 的 `overview` 一次查看最多 50 镜的正文准备状态、在盘图／视频和已绑定人物及角色参考图状态，用 `fields` 只读需要的字段。通过 `shotId` 或 `shotIds` 按 ID 或项目全局镜号（如 `#05`）筛选，批量读取沿用项目顺序。镜头写入也可用这些全局镜号。版本号随成功写入返回。用 `studio_batch_edit_shots` 一次保存最多 100 项输入、关键帧正文或视频定稿。任一项校验失败时整批不写。批量写生图正文用 `edits: [{ "kind": "image-prompt", "shotId": "#05", "visualPrompt": "角色从门外回头" }]`。关键帧正文默认独立，确需同步分镜时传 `syncScript: true`。涉及三个层次时，在同一批里写入分镜、生图和视频各自的正文。时间以 `plannedStartSec`、`plannedEndSec` 为准。既有「镜号｜时间｜标题」格式会随 MCP 时长或顺序写入刷新。

用 `studio_replace_shots` 批量重切分镜。插入时指定已有镜头作为锚点。替换时可指定连续范围，或覆盖整套分镜。每条草稿分别填写场景，复用原镜头 ID 可保留选片、制作设置和画布位置。操作会返回受影响的连续镜头。会移除旧镜时先返回预览，确认后再移除镜头关联，原片仍在素材库。用 `dryRun: true` 可预览任何改动。

准备时列清镜头、所用工作流、已选参考、首尾帧及缺项。用 `studio_describe_model` 的 `workflow.referenceSlots` 读取实际提交顺序（`index` 从 0 开始）和下游端口 `consumers`。仅能确定工作流图片编号时，`pictureNumber` 才有值。首、尾帧分别用 `firstFrameSlot`、`lastFrameSlot`。只有工作流节点明确标注用途时才能根据标签分配角色或场景。确认后提交。普通切镜可以使用资产参考，同场续拍从上一镜选中成片的真实尾帧接续，换场断开。

1. 用原生 ComfyUI 接入的 API 格式工作流。最终提示词可由画布手改，也可用 `studio_set_video_prompt` 写入。生成入口按照当前项目及镜头读到的配置验证。参考图模式可沿用当前人物、场景与道具素材，或为这次生成明确传入参考图。单首帧模式使用明确对应 `first_frame` 的工作流。
2. 用新的请求 UUID 调用 `studio_prepare_video`。返回镜头顺序、提示词、时长、画幅、接入配置、素材和槽位。确认后再用同一请求 ID 调用 `studio_confirm_video`。镜头未保存时，队列会保留当前项并停止推进。
3. 在画布顶部查看批次，或用 `studio_current_video_batch`、`studio_video_status` 查询。镜头有选片时，重跑结果归档为候选。已归档步骤的 `bind.applied` 记录当时是否自动选用。`bind.current` 和 `bind.isCurrent` 反映当前选片。用镜头的历史版本或 `studio_bind_shot_asset` 的 `video` 槽换片，首尾帧可用 `first` 和 `last` 槽选入单幅图。暂停会等当前镜头完成。停止本地等待后，ComfyUI 已收到的任务仍可继续。「停止当前生成」只请求取消弹窗中确认的镜头，需 ComfyUI 支持。确认期间切到下一镜时，重新核对和确认。Agent 代为取消经用户核对的镜头时，向 `studio_cancel_video_shot` 传入该步骤的 `promptId` 作为 `expectedPromptId`。重新打开后，用同一请求 ID 查询并用 `studio_resume_video` 恢复可恢复的任务。未知提交可用 `studio_recover_video_id` 验证原任务归属。旧版 ComfyUI 可传入人工找到的 prompt ID。
4. 已保存的视频出现在分镜和项目素材库。已有选片时，重做的视频进该镜候选。节点检查器可打开原片所在文件夹或复制路径。外部 Agent 用 `studio_locate_asset` 核对原片是否存在。接续镜头保存所用的上镜视频 ID，上一镜更换选片后会提示复核。
- 生成后在镜头和项目素材中核对产物。已有候选可通过 `studio_bind_shot_asset` 选为分镜当前素材。

目前配置与操作以本手册记录的源码构建为准。原版界面补充图文见[上游初次使用教程](教程-第一次用.md)，连续拍的历史交互和时长限制见[上游连续拍说明](连续拍模式.md)。

## ComfyUI 工作流发现

Web 设置的「ComfyUI 接入」可扫描本机常用安装目录和你指定的目录。点发现结果的「查看工作流」，展开权重和可映射输入，选中后导入。需要长期管理或托管启停时再保存实例。已有 API 图可直接导入。界面工作流由所选在线实例的前端自动序列化，Windows 需要 Chrome 或 Edge。所需节点未加载或转换结果不完整时，返回原因并停止。导入到图像或视频配置不会提交生成。端口冲突时先检查占用。在 Windows 上，Dramlo 重启后仍能核实并停止由它启动的 ComfyUI；停止前会检查队列。再次导入同一来源的新版本时保留配置 ID，工作流图有变化时重新推断映射。

用 `studio_comfy_workflows` 浏览。只有一个在线实例时可传空对象。多个在线实例、离线安装或自定义端口先用 `studio_comfy_discover` 找到目标，再传返回的 `target` 或已保存的 `instanceId`。浏览和导入无需登记。将工作流列表项的 `readInput` 原样传给 `studio_comfy_workflow_read` 或 `studio_comfy_import`，可附加各工具的选项。它包含实例定位、来源、相对路径，以及磁盘来源所需的目录索引。`studio_list_models` 只列已保存的模型接入。磁盘和在线目录同路径且内容一致时合并，内容不同的分别显示。优先复用 `profiles` 里的已有配置。导入前核对 `unlinkedProfiles` 中缺少来源记录的旧配置。

### 工作流输入与适配层

ComfyUI API 图以节点 ID 为键，每个节点含 class_type 和 inputs。Dramlo 提交到 /prompt，用返回的 ID 查询 /history。UI 图先由所选 ComfyUI 实例转换。用 studio_comfy_workflow_read 的 includeNodeInputs 读取该实例的节点定义、枚举、范围和明确声明的单位，候选映射只用于辅助核对。

用 studio_comfy_import 的 dryRun 看候选和 workflowFingerprint。确认后带指纹和声明导入。用 studio_model_update 局部更新声明，studio_describe_model 读回。工作流仍保留原图。更新或 patch 后检查 brokenBindings 中的节点、字段和失败原因。

referenceConfig 的槽按 1 开始连续编号。identity 是人物，storyboard 是动作看板，scene 是场景，prop 是道具，reference 是通用素材。referenceAssignments 可跳过可选槽。可选槽使用当前项目指定的中性图补位，避免把模板默认文件当成本镜参考。预检逐槽返回素材 ID 和 fallback 是否生效。视频预检保留专用首尾帧入口。I2VA 会延续首帧的画面结构，整张看板进入首帧会把四格画进视频。要复用一格，用 studio_crop_shot_frame 从当前已选看板本地裁出单幅，再预检视频。

outputConfig 声明可选画幅、目标像素和视频时长范围。每镜可选已声明的 aspectRatio。宽、高映射写入精确像素，ResolutionSelector 使用目标像素量，结果可能按节点规则取整。画幅和角色参考没有凭文件名推断，具体结果以预检回显为准。studio_prepare_comfy_node 的 dryRun 只预检，不会产生可确认请求。

## 本机 Web 工作区

构建 Dramlo 的前端和 Rust 服务后，可用 `startDramlo.cmd <工作区目录> [端口]` 启动，浏览器打开服务打印的回环地址。验收期间使用项目副本。Web 工作区支持项目与模型写入、导入及读取本地媒体、本机 ComfyUI 手动操作和独立制作队列。浏览器与服务在本机通信，项目写入受修订保护。将客户端的 `DRAMLO_WEB_MCP_URL` 设为该回环地址，服务与客户端沿用同一个 `DRAMLO_MCP_BRIDGE_TOKEN`。冷启动可配置 `DRAMLO_WEB_WORKSPACE` 后使用 `studio_start_web`。连接返回的工具列表反映当前运行时能力。Web 版 Dramlo stdio 命令为 `dramlo-server.exe mcp studio`。Rust 在服务端保存批次，关页或结束 Agent 会话后仍可运行。截取连续镜头的真实尾帧、导出成片和合成视频都依赖 FFmpeg。服务按顺序查找：`DRAMLO_FFMPEG_BIN` 指向的可执行文件、程序同级的 `ffmpeg/ffmpeg.exe`、启动脚本从系统 PATH 找到的 `ffmpeg.exe`。发行包运行 `get-ffmpeg.cmd` 可自动下载并放到 `ffmpeg/` 目录：默认走国内镜像（约 29 MB，秒级），镜像不可用时回退官方源（约 120 MB）。装好后无需再配环境变量。Web 服务会拒绝在同一工作区启动第二个实例，即使指定不同端口。发行包用 `npm run package:dramlo-web` 构建，部署只需要其中的程序和前端目录。开发环境用 `npm run test:dramlo:all` 执行隔离测试。
