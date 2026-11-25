# 未接入的内置 Agent 服务层

**这些文件当前没有 UI 入口、不参与构建，不要把它们当成已有能力。**

`c4db3e4`（Move Studio production to a local Web runtime and external MCP agents）把内置 Agent 从界面移除时，只删了展示层，服务层完整留了下来。构建时被 tree-shaking 摇掉，所以不影响运行，但会让读代码的人和外部 Agent 误判「Dramlo 内置了这些能力」。

## 文件清单

| 文件 | 行数 | 作用 |
| --- | --- | --- |
| `services/canvas/agent.ts` | 1326 | 对话主循环、提示词、意图匹配 |
| `services/canvas/toolRegistry.ts` | 1248 | 40 个前端工具的实现 |
| `services/canvas/agentLoop.ts` | 276 | 执行循环 |
| `services/canvas/aiSdkAgent.ts` | 216 | 工具 schema 与模型调用 |
| `services/canvas/agentMedia.ts` | 123 | 看图与尾帧续接 |
| `services/canvas/actionPolicy.ts` | 77 | 动作去重、防重复计费 |
| `services/canvas/planner.ts` | 63 | 计划结构 |
| `components/canvas/AgentTraceList.vue` | 283 | 执行轨迹展示 |
| `services/canvas/agentTrace.ts` | 13 | 轨迹类型 |

合计 3625 行。

## 与 MCP 的关系

这 40 个工具里约 22 个已在服务端有等价能力（`studio_*` 前缀），**服务端是唯一可用的那一份**：

```
seed_script          → studio_import_script
revise_shot          → studio_revise_shot
update_node          → studio_set_asset_prompt / studio_batch_edit_shots
generate_*（生图）    → studio_prepare_image + studio_confirm_image
generate_video       → studio_prepare_video + studio_confirm_video
check_project        → studio_project_overview / studio_check_video_readiness
```

两侧语义不同：前端直接改内存 store，服务端带 `expectedUpdatedAt` 乐观锁。**接入时必须只走服务端，否则会绕过版本校验。**

## 服务端没有对应能力的部分

```
generate_play / continue_play / generate_storyboard / extract_cast
optimize_node_prompt
```

这些是「调模型生成文本」，服务端不做 LLM。正确的分工是：调用方模型自己生成，再用 `studio_update_script`、`studio_append_shots`、`studio_upsert_project_assets` 落盘。

## 接入时注意

1. 需要重建对话面板（原 `AgentChat.vue` 已删）
2. 工具调用改为走 MCP，不要直接调 `toolRegistry` 里的实现
3. 行为准则（默认动手、大额确认、验收闸门）见 `docs/studio-operation-manual.md`
