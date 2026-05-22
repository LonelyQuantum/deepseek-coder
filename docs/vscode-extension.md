# 编辑器插件（VS Code Extension）

状态：草案。

VS Code 插件是 `deepseek-coder` 的一等前端。它应通过 JSON-RPC server 复用 Rust Agent Core。

## 职责

- 启动并监管 Agent RPC Server。
- 渲染 chat 和 run events。
- 展示计划、tool call 和审批请求。
- 使用 VS Code 原生 diff editor 展示 patch。
- 从 Problems 面板读取 diagnostics。
- 尊重 Workspace Trust。
- 暴露 provider、model 和 approval policy 设置。

## 非职责

插件不应实现自己的 Agent loop、context builder 或 tool execution engine。

## 当前骨架

当前插件只注册 `deepseek-coder.openChat`，并提示 workspace 已就绪。RPC 集成尚未实现。

## MVP 分层

VS Code 插件的短期目标是成为 Agent Core 的薄前端，而不是追赶成熟通用插件的全部功能。

Phase 1/早期 Phase 4 的顺序：

1. 启动并监管 Rust Agent RPC Server。
2. 渲染 `agent.event` 事件流。
3. 展示审批请求和命令输出摘要。
4. 使用 VS Code 原生 diff editor 展示 patch。
5. 读取 Problems 面板诊断并交给 Agent Core。

在这些能力稳定前，不在插件侧重复实现 context builder、tool execution 或 provider 调用。

## 后续增强

- 启动并监管 Rust Agent RPC Server，处理进程退出、版本不匹配和 workspace trust。
- 渲染 `agent.event` 流，包括 assistant delta、计划、工具调用、审批请求、patch 和验证结果。
- 使用 VS Code 原生 diff editor 展示 patch，并支持 hunk 级审批。
- 从 Problems 面板读取 diagnostics，并通过协议传给 Agent Core，而不是在插件内自行生成修复逻辑。
- 提供 provider、model、base URL、审批策略和上下文预算设置，同时保证 API Key 只走安全存储或用户显式配置。
- 增加 `@vscode/test-electron` 集成测试，覆盖命令注册、RPC 启动失败提示和基本事件渲染。
