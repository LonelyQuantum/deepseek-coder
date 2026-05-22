# 设计文档

本目录保存 `deepseek-coder` 的详细设计。

仓库根目录的 `README.md` 继续作为项目入口和高层路线图。具体子系统设计放在这里，便于各模块独立演进。

各技术文档应在对应模块下记录当前实现范围、尚未实现的部分和后续增强项。README 只保留跨模块的大路线，避免把细节计划散落在项目入口。

文档正文默认使用中文。代码里的协议字段、事件名、错误码、模型参数、provider-facing tool description 和 crate/package 标识可以保留英文；面向中文用户的 UI 文案可以使用中文。后续如果同一段工具描述需要同时服务 provider schema 与本地 UI，应从统一注册表生成，避免 Rust 与 TypeScript 手工维护两套语义。

## 文档索引

- `architecture.md`：总体架构和 workspace 布局。
- `roadmap.md`：跨模块路线图、阶段优先级和验收重点。
- `agent-core.md`：Agent Core 职责和回合生命周期。
- `deepseek-api-adapter.md`：DeepSeek API adapter 设计。
- `reasoning-content.md`：`reasoning_content` 状态机。
- `json-rpc-protocol.md`：前端与 Rust RPC Server 之间的内部协议。
- `run-log.md`：本地运行日志、事件存储和回放基础。
- `context-capsule.md`：长上下文构建和缓存友好的 prompt 布局。
- `tool-system.md`：内置工具注册、schema 和执行结果格式。
- `approval-model.md`：风险等级和审批流程。
- `vscode-extension.md`：VS Code 插件设计。
- `tui.md`：终端 UI 设计。
- `security-model.md`：安全边界和威胁模型。
- `release.md`：发布、许可证和分发策略。

## 架构决策记录

ADR 位于 `adr/`。它们记录已经接受的重要决策，以及这些决策背后的原因。
