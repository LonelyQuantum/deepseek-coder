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
- `crates/agent-rpc`：JSON-RPC server 和协议桥接。
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
- `agent-core` 已包含 provider adapter、流式解析、`reasoning_content` 状态机、工具/审批基础类型、read/search/apply_patch/shell/git 基础执行层、基础 run log、基础 Context Builder 和基础 Agent Turn Loop。
- `agent-rpc`、CLI 和 TUI 仍是骨架；真实 provider streaming 接入、RPC server 和 CLI 最小闭环尚未实现。

## 后续增强

- 把 Agent Core 的基础 Turn Loop 接入真实 provider streaming、RPC server、CLI 和验证命令。
- 实现 `crates/agent-rpc`，让 CLI/TUI/VS Code 通过同一套 JSON-RPC 协议调用 Agent Core。
- 明确 `.deepseek-coder/` 本地状态的目录结构、版本迁移策略和脱敏规则。
- 增加端到端测试，覆盖 CLI/TUI/VS Code 对同一任务产生一致 run log 的能力。
