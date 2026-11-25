# Dramlo 文档

从这里找文档。产品范围与验收以[产品目标与验收基线](studio-product-goal.md)为准。

| 分类 | 文档 | 更新时机 |
| --- | --- | --- |
| 产品 | [产品目标与验收基线](studio-product-goal.md) | 确认需求、技术方向、优先级、验收结果或范围变化时。 |
| 产品故事 | [为什么做 Dramlo｜使用经历与宣传素材](studio-origin-and-user-experience.md) | 补充亲身遇到的问题、解决思路和可演示的宣传素材。 |
| 设计 | [Dramlo 设计指引](studio-design-guide.md) | 补充体验问题、设计取舍、改进顺序和验收场景时。 |
| 扩展设计 | [Dramlo 扩展库与导入规范草案](studio-extension-library.md) | 扩展管理、节点贡献、导入和开发规范调整时。 |
| 用户 | [Dramlo 操作手册](studio-operation-manual.md) · [外部 Agent 接入指南](studio-operation-manual.md#3-接入外部-agent) | 界面、配置、MCP 操作、结果反馈和故障接手方式变化时。 |
| 工程 | [内置 Agent 盘点与运行时迁移清单](studio-agent-migration.md) | 源码职责、技术迁移、测试或兼容性变化时。 |
| 历史 | [早期方案归档](archive/early-plans.md) | 追溯已替代的设计方案。新需求在产品基线维护。 |

## 上游参考

上游教程与说明保留原路径，便于引用和同步。它们反映原版或早期功能，使用 Dramlo 时请查上面的操作手册和验收基线。

- 操作与素材。[第一次用教程](教程-第一次用.md)、[连续拍模式](连续拍模式.md)、[图库模板生成说明](图库模板生成说明.md)。
- 开发与安全。[原版架构](../ARCHITECTURE.md)、[功能清单](../FEATURES.md)、[贡献指引](../CONTRIBUTING.md)、[安全说明](../SECURITY.md)。
- 更新与版本。[更新日志与上游历史](../CHANGELOG.md)、[上游更新说明](../更新说明.md)。

## Skill 维护目录

[通用 AI 短剧创作 Skill 2.0](../src/skills/drama-flow-explore/SKILL.md) 覆盖全剧分集与逐集制作。参考手册与关键镜组示范可通过技能库或 MCP 按需读取。各册的规则直接标注所据原文与适用限制。[来源与覆盖记录](../src/skills/drama-flow-explore/references/source-index.md)列明原文范围、出典与适用边界。

[Codex Dramlo 短剧创作 Skill](../src/skills/codex-short-drama/SKILL.md) 收录九份按工序选读的手册，涵盖编剧、分镜、资产、看板、静帧、视频终稿和验收。

- [src/skills](../src/skills) 维护按制作环节选择的创作 Skill。画布与 Rust MCP 从这里编译正文。`studio-workbench` 和 `studio-comfy-workflows` 是技术接入入口。工具能力和参数由 MCP 清单维护，操作步骤和确认边界见操作手册。

项目文档的变更约定见 [AGENTS.md](../AGENTS.md)。
