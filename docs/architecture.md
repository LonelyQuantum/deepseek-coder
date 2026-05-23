# 总体架构

状态：草案，Phase 1 部分实现。

`deepseek-coder` 分为 Rust 核心和 TypeScript 前端/共享包。

```text
CLI/TUI                 VS Code Extension
   |                           |
   +----------- JSON-RPC ------+
               |
        Agent RPC Server
               |
          Agent Core
               |
   +-----------+-----------+
   |           |           |
Context     Tools      Providers
Builder   Registry    DeepSeek API
```

## 目标

- CLI、TUI 和 VS Code 共用同一个 Agent Core。
- UI 层保持轻量，只负责渲染、用户输入、审批提示和编辑器集成。
- 工具执行和审批策略由 Rust 侧统一管理。
- 协议类型显式、版本化、可测试。
- 本地状态可检查、可复现、可审计。

## 工作区布局（Workspace）

Rust workspace：

- `crates/agent-core`：turn loop、上下文构建、工具编排、run log。
- `crates/agent-rpc`：JSON-RPC server、stdio framing 和协议桥接。
- `crates/cli`：命令行入口。
- `crates/tui`：终端 UI 入口。

TypeScript workspace：

- `packages/protocol`：共享 TypeScript 协议类型。
- `vscode/extension`：VS Code 插件实现。

## 本地状态

运行时状态默认保存在 `.deepseek-coder/`：

- 配置快照
- workspace manifest
- run log
- token 和 cache 使用摘要
- 审批记录

密钥不得写入 run log。

## 当前实现

- Rust workspace、TypeScript workspace、VS Code 插件骨架和共享协议包已建立。
- `agent-core` 已包含 provider adapter、流式解析、streaming tool call delta accumulator、`reasoning_content` 状态机、工具/审批基础类型、read/search/apply_patch/shell/git 基础执行层、基础 run log、基础 Context Builder、基础 Agent Turn Loop、async / streaming `TurnProvider` 边界和 `TurnEventSink` 实时事件出口。
- `agent-rpc` 已实现 Run Log 事件到 `agent.event` JSON-RPC notification 的基础 stdio 桥接，`StdioEventBridge` 可直接作为 `TurnEventSink` 使用；同时已实现 `agent.initialize` / `agent.sendTurn` / `agent.approve` / `agent.reject` / `agent.resume` 的双向 request loop。真实 Turn Loop handler 和 RPC pending approval 队列尚未实现。
- CLI 已实现 `run` 最小闭环，能直接调用 Agent Core、通过 DeepSeek streaming wrapper 驱动真实 provider，在 `--json` 模式下随着 run log 写入实时输出 JSON-RPC event，并支持 stdin/stderr 交互式审批；TUI 已有审批 prompt 状态机但仍未接入完整 ratatui 界面；VS Code 插件已有 modal approval adapter 但仍未接入真实 RPC server。

## 后续增强

- 补齐真实 RPC Turn Loop handler、RPC 审批等待队列，并把 TUI/VS Code 审批原语接入真实 UI。
- 扩展 `crates/agent-rpc`，让 CLI/TUI/VS Code 通过同一套 JSON-RPC 协议调用 Agent Core。
- 明确 `.deepseek-coder/` 本地状态的目录结构、版本迁移策略和脱敏规则。
- 增加端到端测试，覆盖 CLI/TUI/VS Code 对同一任务产生一致 run log 的能力。
