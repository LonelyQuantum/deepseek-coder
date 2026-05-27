# 编辑器插件（VS Code Extension）

状态：Phase 3 优先开发项。基础命令、审批弹窗 adapter、RPC server 启动监管、初始化握手和 JSON-RPC request client 已实现；尚未实现完整 Chat UI、事件渲染、真实审批回传和 diff editor 集成。

VS Code 插件是 `ProleCoder` 的一等前端。它必须通过 JSON-RPC server 复用 Rust Agent Core，而不是在 TypeScript 侧重新实现 agent loop、context builder、provider 调用或 tool execution。

## 职责

- 启动并监管 Rust Agent RPC Server。
- 渲染 chat 和 run events。
- 展示计划、tool call、审批请求和命令输出摘要，并通过 `agent.approve` / `agent.reject` / `agent.cancel` 回传用户决定。
- 使用 VS Code 原生 diff editor 展示 patch。
- 从 Problems 面板读取 diagnostics。
- 尊重 Workspace Trust。
- 暴露 provider、model、RPC 命令和审批策略设置。

## 非职责

插件不实现自己的 Agent loop、context builder、tool execution engine 或 provider adapter。插件只是前端和进程监管层，事实来源仍是 Rust RPC server 与 Run Log。

## 当前实现

`vscode/extension/src/rpcServer.ts` 提供 `RpcServerManager`：

- 通过可配置命令启动 Rust RPC server，默认命令为 `prole`，默认参数为 `rpc`。
- 启动后立即发送 `agent.initialize`，携带 `protocolVersion`、`client.frontend = "vscode"`、`workspaceRoot` 和 `workspaceTrusted`。
- 按行解析 stdout 上的 JSON-RPC response / notification。
- 把 `agent.event` notification 转发给注册的事件 handler。
- 通过 `sendRequest()` 发送 JSON-RPC request，并按 request id 管理 pending response。
- 把 JSON-RPC error response 转换为 `RpcRequestError`，保留 `code` 和 `data`。
- server 停止、退出或出错时，会拒绝尚未完成的 pending request。
- 记录 stderr 尾部，供后续错误提示和诊断使用。
- 如果 server 在 ready 后意外退出，状态进入 `failed` 并提示用户。
- 插件 dispose 时关闭 stdin 并 kill 子进程。
- 未受信任 workspace 不会启动 server。

`vscode/extension/src/commands.ts` 当前注册 `prole-coder.openChat`：

- 如果没有 workspace，则提示先打开 trusted workspace。
- 如果有 RPC manager，则尝试启动或复用 RPC server，并提示 server ready 或启动失败。

`vscode/extension/src/commands.ts` 还提供 `requestApproval`：

- 使用 VS Code modal warning 展示审批摘要。
- 将 `Approve` / `Approve Once` / `Approve For Session` / `Reject` / 关闭弹窗映射为稳定的批准或拒绝决定。
- 后续可直接用于把 `tool.approvalRequired` 事件转换为 `agent.approve` / `agent.reject` request。

## 配置

```json
{
  "prole-coder.rpc.autoStart": true,
  "prole-coder.rpc.command": "prole",
  "prole-coder.rpc.args": ["rpc"]
}
```

开发时如果本机尚未安装 `prole` 可执行文件，可以把命令设置为 `cargo`，参数设置为：

```json
{
  "prole-coder.rpc.command": "cargo",
  "prole-coder.rpc.args": ["run", "-p", "prole-coder-cli", "--", "rpc"]
}
```

配置不保存 API Key。DeepSeek API Key 仍应由 Rust CLI/RPC server 按既有规则从环境变量或被忽略的本地 `.secrets/` 文件读取。

## MVP 分层

短期目标是让 VS Code 插件成为 Agent Core 的薄前端，而不是追赶成熟通用插件的全部功能。

Agent Core MVP 只要求 Rust Core / RPC server 提供稳定协议和事件流；VS Code 插件工作从 Phase 3 开始成为主要交付物。TUI 继续保留，但排在 VS Code 核心体验之后。

Phase 3 P0 顺序：

1. 启动并监管 Rust Agent RPC Server。已完成基础实现。
2. 渲染 `agent.event` 事件流。当前 manager 已能转发事件，但 UI 尚未消费。
3. 支持文本输入并通过 `agent.sendTurn` 发送真实 turn。
4. 通过 JSON-RPC request client 回传用户动作。当前已完成通用 `sendRequest()`，尚未接入具体 UI。
5. 展示审批请求和命令输出摘要。当前已有 modal approval adapter，尚未接入真实 RPC 事件。
6. 使用 VS Code 原生 diff editor 展示 patch，并为 hunk 级审批预留交互边界。
7. 展示 Run List / resume 和 Context Capsule 可视化。

Phase 4 P1/P2 深度集成：

1. 读取 Problems 面板诊断并交给 Agent Core。
2. Terminal command approval 展示命令、cwd、风险等级、输出摘要和持久化选项。
3. provider、model、预算、审批策略和 RPC 命令配置界面。
4. FIM completion preview。
5. VSIX alpha / pre-release 打包与安装说明。

在这些能力稳定前，不在插件侧重复实现 context builder、tool execution 或 provider 调用。

## 后续增强

- 把 `tool.approvalRequired` notification 接入 `requestApproval`，并把结果发送为 `agent.approve` / `agent.reject`。
- 为 `agent.sendTurn`、`agent.approve`、`agent.reject`、`agent.cancel` 等常用方法增加类型化 helper，避免 UI 层直接拼 method string。
- 支持 `agent.cancel`，在用户关闭 run 或插件停用时取消 pending run。
- 处理协议版本不匹配：显示 server/client protocol version，并引导用户升级对应组件。
- 支持多 workspace folder：每个 workspace root 对应一个 RPC server 或明确选择 active workspace。
- 渲染 `agent.event` 流，包括 assistant delta、计划、工具调用、审批请求、patch 和验证结果。
- 使用 VS Code 原生 diff editor 展示 patch，并支持 hunk 级审批。
- 从 Problems 面板读取 diagnostics，通过协议传给 Agent Core，而不是在插件内自行生成修复逻辑。
- 增加 `@vscode/test-electron` 集成测试，覆盖真实 extension activation、配置读取、启动失败提示和基础事件渲染。
