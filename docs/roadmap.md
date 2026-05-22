# 路线图

状态：草案，随 Phase 1 实现持续更新。

本文档把 README 中的大阶段拆成更可执行的优先级。README 保留项目入口和高层计划；这里记录跨模块的落地顺序、取舍和验收重点。

## 定位

`deepseek-coder` 不以第一阶段覆盖通用 AI 编程工具的全部功能为目标。项目的核心差异是：

- DeepSeek V4 1M 上下文优先。
- `reasoning_content`、上下文缓存和长输出能力一等支持。
- Rust Agent Core 供 CLI/TUI/VS Code 共用。
- 可复现 run log、显式审批和自由软件治理。
- 中文工作流友好。

因此短期不和成熟 VS Code 扩展正面拼功能覆盖率，而是先做出一个可审计、可复现、能稳定闭环的小型 Agent。

## P0：Phase 1 MVP 收敛

目标：完成一个最小但真实可用的 Agent 回合。

已完成：

- DeepSeek API adapter。
- 流式响应解析。
- `reasoning_content` 状态机。
- read/search/apply_patch/shell/git 基础工具执行层。
- 基础 Run Log 存储层。

下一步：

- 基础 Context Builder：读取任务、项目规则、git 状态、必要文件和工具结果，并记录 token 预算来源。
- Agent Turn Loop：串联 provider streaming、tool call 收集、schema 校验、审批请求、工具执行、run log 写入和继续请求。
- Agent RPC Server 最小 stdio 桥接：把 run events 通过 JSON-RPC notification 传给前端。
- CLI 最小闭环：`deepseek-coder run "<task>"` 能在小型仓库中执行一次“读取 -> 修改 -> 验证 -> 报告”。
- 端到端 smoke test：使用 fake provider 或本地 fixture，验证 turn loop、工具、run log 和 CLI/RPC 事件一致。

P0 不追求：

- 完整 TUI。
- 完整 VS Code Sidebar。
- MCP 生态。
- 多 provider UI。
- 大仓库 1M token 基准。

这些功能应在核心闭环可复现之后再推进。

## P1：编辑器核心体验

目标：让 VS Code 插件成为 Agent Core 的薄前端，而不是第二套 Agent。

优先事项：

- 插件启动并监管 Rust Agent RPC Server。
- Sidebar Chat 渲染 run events。
- 原生 diff editor 展示 patch。
- Problems 面板诊断进入 Context Builder。
- Terminal command approval 展示命令、cwd、风险等级和输出摘要。

验收重点：

- 插件和 CLI 对同一任务产生一致 run log。
- 插件不直接实现 tool execution、context builder 或 turn loop。

## P2：DeepSeek 差异化

目标：把 DeepSeek V4 的长上下文和思考模式变成可见、可审计的工作流。

优先事项：

- 1M Context Capsule。
- 稳定前缀与缓存命中统计。
- token 预算报告。
- `reasoning_content` replay 状态摘要。
- FIM completion preview。

验收重点：

- 能解释哪些内容进入上下文、哪些没有进入，以及原因。
- 能记录 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` 并展示给前端。

## P3：生态扩展

目标：在核心闭环、编辑器体验和 DeepSeek 差异化稳定后，再扩展通用能力。

候选事项：

- MCP client。
- 本地模型或私有推理服务适配器。
- 包管理器工具。
- issue/PR 工具。
- 审计包导出。
- 可复现发布和多渠道安装。

## 风险控制

- 避免只堆文档和抽象：每个阶段都应有可运行验收场景。
- 避免前端行为分叉：CLI/TUI/VS Code 必须共享 Agent Core 和 run log。
- 避免过早追求通用多模型：provider API 要保持表达性，但 DeepSeek 是首个落地目标。
- 避免把 1M 上下文当作兜底：长上下文必须配合 token 预算、来源标注和验证命令。
