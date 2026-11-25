# Dramlo

首个独立版本为 **0.0.1**，目前尚未发布。

**用你熟悉的 Agent 和喜欢的模型，聊着把短剧做出来。**

Dramlo 是可本地部署的开源 AI 短剧制作工作台。只要你惯用的 Agent 平台能连接 Dramlo 的 MCP，就能在原来的对话里让 AI 主导制作，把剧本、分镜、素材和进度写回画布。你也可以直接打开工作台，从头到尾手动制作。两套流程共用同一个工程，过程中随时可以接手。

## 两套制作流程

- **Agent 主导。** 在你熟悉的 Agent 平台里讨论剧情、调整分镜、确定参考和审看结果。Agent 使用你在该平台选的模型，通过 MCP 操作 Dramlo 的画布和项目，按需要预检和提交生成。修改和产物会回到工作台。
- **全程手动。** 在 Dramlo 里导入或编辑剧本，整理人物和场景，拆分镜、改提示词、选参考、生成图片和视频，再剪辑、加字幕和导出。也可以接着 Agent 已完成的项目继续做。

## 本地部署与生成接入

- **本地或离线工作。** Dramlo 的 Web 界面、制作服务、项目和素材可以留在本机。使用本地 Agent、本地模型和已准备好权重与节点的 ComfyUI，可组成离线制作环境。选用在线 Agent 或生成 API 时，相应请求会走所配置的服务。
- **直接接入 ComfyUI。** 从 Dramlo 扫描本机实例、浏览已保存的工作流，查看节点与输入槽，再直接导入配置。常规接入无需自己在 ComfyUI 里手动导出 API JSON。Agent 也可以通过同一个 MCP 连接完成发现和导入。
- **可自配在线图像和视频 API。** 在设置中分别选择或添加供应商，填写 API 地址、密钥和模型。创作 Agent 使用的模型由你在 Agent 平台里选择，实际出图、出片使用 Dramlo 项目选用的工作流或 API。

Dramlo 面向有本地模型或 ComfyUI 环境的创作者、希望自行部署和管理数据的团队，以及想继续使用自己熟悉的 Agent 平台的用户。企业也可以在自己的环境部署制作服务、管理工作区，并接入内部模型或已批准的供应商。

## 内置短剧流程与 Skill

内置专业短剧制作流程及全流程创作 Skill，覆盖编剧、分镜、人物和场景资产、故事看板、图像与视频提示词、成片验收。Agent 按工序通过 MCP 读取所需 Skill 和参考资料。手动制作也可以按同一流程推进。

## 三步接入 Agent

1. 拿到 `Dramlo-Web` Windows 构建包后，解压并运行 `start.cmd`，打开终端打印的本机地址。公开下载入口将在首个版本发布后补上。
2. 打开「设置 → 外部 Agent 接入」，点「复制连接说明给 Agent」，把整段提示词发到你惯用的 Agent 平台。提示词带有当前工作台的连接参数。客户端需要支持加载 MCP，首次配置按客户端要求授权或启用连接。
3. 确认连通后，直接告诉 Agent 你想怎么做。比如「读我导入的剧本，先拆第一场的分镜并写回画布，等我看完再开始生成」。 [接入与排查](docs/studio-operation-manual.md#3-接入外部-agent)

## 画布

<!-- 此处预留给优化后的画布介绍、模型说明和演示图。 -->

## 开发版构建

以下命令从仓库源码构建 Dramlo。

Windows 开发环境需要 Node.js、npm 和 Rust。导出成片或提取视频尾帧时需要 FFmpeg。

```powershell
npm ci
npm run build
cargo build --release --locked --manifest-path dramlo-server/Cargo.toml
.\startDramlo.cmd "D:\DramloWorkspace"
```

运行 `npm run package:dramlo-web` 可构建 Windows 发布包，输出位于 `release-pack/`。

## 项目文档

[文档索引](docs/README.md) · [操作手册](docs/studio-operation-manual.md) · [更新日志](CHANGELOG.md)

## 许可证与致谢

Dramlo 独立维护，采用 [MIT 许可证](LICENSE)。感谢 [AI-Short-Drama-Studio](https://github.com/JerrIsTheBesta/AI-Short-Drama-Studio) 的原始贡献。
