# 设计文档

本目录保存 `deepseek-coder` 的详细设计。

仓库根目录的 `README.md` 继续作为项目入口和高层路线图。具体子系统设计放在这里，便于各模块独立演进。

## 文档索引

- `architecture.md`：总体架构和 workspace 布局。
- `agent-core.md`：Agent Core 职责和回合生命周期。
- `json-rpc-protocol.md`：前端与 Rust RPC Server 之间的内部协议。
- `context-capsule.md`：长上下文构建和缓存友好的 prompt 布局。
- `tool-system.md`：内置工具注册、schema 和执行结果格式。
- `approval-model.md`：风险等级和审批流程。
- `vscode-extension.md`：VS Code 插件设计。
- `tui.md`：终端 UI 设计。
- `security-model.md`：安全边界和威胁模型。
- `release.md`：发布、许可证和分发策略。

## 架构决策记录

ADR 位于 `adr/`。它们记录已经接受的重要决策，以及这些决策背后的原因。
