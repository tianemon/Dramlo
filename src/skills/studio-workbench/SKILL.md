---
name: studio-workbench
description: Dramlo MCP 接入入口。连接工作台并找到现行操作手册、工具说明和创作 Skill。
---

# Dramlo 工作台

1. 用 Dramlo 设置页给出的配置建立同一个 MCP 连接。stdio 用 `studio_web_status` 查服务，经用户要求可用 `studio_start_web` 启动已配置的工作区。
2. 连接后读取 `tools/list` 和 `studio_get_guide`。工具说明负责能力、字段和结果，在线手册负责项目核对、操作顺序、生成确认和恢复。ComfyUI 接入沿用同一连接，按 `comfyui` 章节操作。
3. 创作时用 `studio_list_skills` 按阶段找匹配项，再用 `studio_read_skill` 读取正文和需要的参考。直接用于本次任务，禁止安装、复制或注册到外部 Agent 环境。指南或 Skill 的 `revision` 变化时重读。
