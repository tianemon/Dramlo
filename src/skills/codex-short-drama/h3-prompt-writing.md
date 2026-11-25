---
name: h3-prompt-writing
description: 为 Dramlo 分镜编写 MiniMax H3 出片正文，按人物参考、首尾帧或纯文本选择结构。
triggers: H3视频,H3 视频,H3出片,Ref2VA,R2VA,I2VA,L2VA,FL2VA,T2VA
---

# H3 视频终稿

先读取本镜采用的工作流、输入用途、实际传入顺序和目标时长。按执行绑定选模式。确认正向提示词是否直达生成节点，经过扩写或翻译时核对实际生效的正文。

| 模式 | 实际输入 | 终稿要求 |
| --- | --- | --- |
| T2VA | 纯文本 | 直接写三段，建立可见主体、环境与初态。 |
| I2VA | 单首帧 | 首行对齐起始画面，空一行后接三段，从给定状态继续。 |
| L2VA | 单尾帧 | 首行对齐落幅，空一行后接三段，选择可自然抵达该状态的起点。 |
| FL2VA | 首帧与尾帧 | 首行分别对齐两端，空一行后接三段，描述两端之间可实现的变化。 |
| Ref2VA | 人物、地点、动作等参考 | 写六段，显式说明每份实际参考约束什么。普通参考槽只承担其声明的用途。 |

## 三段格式

按下列字段顺序输出。`integrated_multimodal_description` 从 `[Shot 1]` 开始，交代风格、景别、机位、主体位置与初态，再按播放次序写触发、表演、摄影、对白和落幅。`overall_soundscape` 写环境底声、动作同步声和声源变化。`non_diegetic_music` 写观众配乐；无配乐或已交后期时填 `N/A`。

使用对应的端点首行。将 `N` 替换为最后一个内部镜号，`S.SS` 替换为目标片段的实际结束时间，以秒为单位保留两位小数。

- I2VA：`For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.`
- L2VA：`How the reference pictures align with the target video — <Picture 1> (from [Shot N]) aligns with the S.SS-second mark of the target video.`
- FL2VA：`How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot N) aligns with the S.SS-second mark of the target video.`

## 六段格式

| 输出顺序 | 填写内容 |
| --- | --- |
| `subject_definitions` | 给需要独立追踪的主体或参考逐项定义，标明实际素材序号、可见特征及用途。 |
| `summary` | 用与实际输入关系一致的方括号任务类型开头，概述目标事件并沿用已定义的标签。一般参考生成用 `[reference generation]`。 |
| `retention_analysis` | 每个已定义对象一行，说明在哪些镜内保留或改变哪些特征。视觉关系选 `fully_preserved`、`partially_preserved`、`attribute_transfer` 或 `weak_reference`。音频关系选 `fully_copy`、`partially_copy`、`reference` 或 `weak_reference`。 |
| `detailed_description` | 先以一两句确立风格，再从 `[Shot 1]` 按时间顺序写表演、空间、机位、声音及参考实际生效的位置。 |
| `overall_soundscape` | 写可听环境及其与动作、切点的同步关系。 |
| `non_diegetic_music` | 写观众能听到的配乐或 `N/A`。角色能听见的音乐写在镜内。 |

## 编号、语言与交稿核对

动作与声音用英文写，台词、歌词及须入画的文字保留定稿原文。`<Picture N>` 对应实际图片顺序，`<Subject N>` 标识被跟踪对象，`(Sx)` 标识声源。官方写法按目标片首次发声顺序分配声源编号，同一声源跨切复用。若 Dramlo 预检附有 `Speaker lock`，逐人核对实际图与发声人对应关系，消除编号冲突再提交。

把说话人、动作与语气写在对白标签外，标签采用 `<d>[Language] 定稿原词</d>`。画外声标明声源；后期独立配音只在画面稿中保留表演窗口。无对白时省略对白标签，按剧情区分环境声、非语言人声与旁白。

仅在发生实际剪切时另起 `[Shot N] At MM:SS.mmm, ...`。连续运动在同一镜号内描述节奏变化。提交前替换所有占位符，核对切点早于片尾、动作能接到下一镜、每个素材编号都有真实输入、声音各段彼此相容。输出工作流所收的终稿格式。完整模式约定见[基础模式指南](https://huggingface.co/MiniMaxAI/MiniMax-H3/blob/main/docs/VIDEO_PROMPT_WRITING_GUIDE_base_en.md)和[全参考指南](https://huggingface.co/MiniMaxAI/MiniMax-H3/blob/main/docs/VIDEO_PROMPT_WRITING_GUIDE_ref_en.md)。
