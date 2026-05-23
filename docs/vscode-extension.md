# 编辑器插件（VS Code Extension）

状态：草案。

VS Code 插件是 `deepseek-coder` 的一等前端。它应通过 JSON-RPC server 复用 Rust Agent Core。

## 职责

- 启动并监管 Agent RPC Server。
- 渲染 chat 和 run events。
- 展示计划、tool call 和审批请求，并通过 `agent.approve` / `agent.reject` 回传用户决定。
- 使用 VS Code 原生 diff editor 展示 patch。
- 从 Problems 面板读取 diagnostics。
- 尊重 Workspace Trust。
- 暴露 provider、model 和 approval policy 设置。

## 非职责

插件不应实现自己的 Agent loop、context builder 或 tool execution engine。

## 当前骨架

当前插件只注册 `deepseek-coder.openChat`，并提示插件脚手架已就绪、真实 agent run 仍需先使用 CLI。RPC 集成尚未实现。

## MVP 分层

VS Code 插件的短期目标是成为 Agent Core 的薄前端，而不是追赶成熟通用插件的全部功能。

Phase 1/早期 Phase 4 的顺序：

1. 启动并监管 Rust Agent RPC Server。
2. 渲染 `agent.event` 事件流。
3. 展示审批请求和命令输出摘要；审批 UI 消费 `tool.approvalRequired`，并以后续 `tool.approvalResolved` 更新状态。
4. 使用 VS Code 原生 diff editor 展示 patch。
5. 读取 Problems 面板诊断并交给 Agent Core。

在这些能力稳定前，不在插件侧重复实现 context builder、tool execution 或 provider 调用。

## 后续增强

- 启动并监管 Rust Agent RPC Server，处理进程退出、版本不匹配和 workspace trust。
- 渲染 `agent.event` 流，包括 assistant delta、计划、工具调用、审批请求、patch 和验证结果。
- 对 `tool.approvalRequired` 使用 VS Code modal warning / quick pick 展示 `toolName`、风险、命令/路径和持久化能力，并发送 `agent.approve` / `agent.reject`。
- 使用 VS Code 原生 diff editor 展示 patch，并支持 hunk 级审批。
- 从 Problems 面板读取 diagnostics，并通过协议传给 Agent Core，而不是在插件内自行生成修复逻辑。
- 提供 provider、model、base URL、审批策略和上下文预算设置，同时保证 API Key 只走安全存储或用户显式配置。

## 当前实现

`vscode/extension/src/commands.ts` 已提供 `requestApproval`：

- 使用 VS Code modal warning 语义展示审批摘要。
- 将 `Approve` / `Approve Once` / `Approve For Session` / `Reject` / 关闭弹窗映射为稳定的批准或拒绝决定。
- 为后续 RPC 客户端发送 `agent.approve` / `agent.reject` 准备好 `approvalId`、`persist` 和拒绝原因。

当前插件尚未启动真实 RPC server，也还没有把 `tool.approvalRequired` 事件自动接入该 adapter。

下一步：

- 启动并监管 Rust Agent RPC Server，处理进程退出、版本不匹配、workspace trust 和启动失败提示。
- 把 `tool.approvalRequired` notification 接入 `requestApproval`，并把结果发送为 `agent.approve` / `agent.reject`。
- 增加 `@vscode/test-electron` 集成测试，覆盖命令注册、RPC 启动失败提示和基本事件渲染。
