# 编辑器插件（VS Code Extension）

状态：Phase 3 优先开发项。基础命令、审批弹窗 adapter、RPC server 启动监管、初始化握手、JSON-RPC request client、VS Code/protocol TypeScript 类型共享、RPC/commands 边界测试、Sidebar Chat 事件渲染、Chat 输入发送真实 turn、真实审批回传、共享 RPC 全双工事件管线、命令风险动态升级展示、Native diff editor patch 预览、Run List / resume 和 Context Capsule 可视化已实现。

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
- 提供 typed `sendTurn()`、`approve()` 和 `reject()` helper，避免 UI 层直接拼常用 JSON-RPC method string。
- 把 JSON-RPC error response 转换为 `RpcRequestError`，保留 `code` 和 `data`。
- server 停止、退出或出错时，会拒绝尚未完成的 pending request。
- 记录 stderr 尾部，供后续错误提示和诊断使用。
- 从 `@prole-coder/protocol` 复用 `AgentEventEnvelope` 类型，避免 extension 本地重复定义事件 envelope。
- 如果 server 在 ready 后意外退出，状态进入 `failed` 并提示用户。
- 插件 dispose 时关闭 stdin 并 kill 子进程。
- 未受信任 workspace 不会启动 server。

`vscode/extension/src/commands.ts` 当前注册 `prole-coder.openChat`：

- 如果没有 workspace，则提示先打开 trusted workspace。
- 如果有 RPC manager，则聚焦 ProleCoder Chat view，尝试启动或复用 RPC server，并提示 server ready 或启动失败。

`vscode/extension/src/chatView.ts` 当前注册 `prole-coder.chat` Webview view：

- 在 Activity Bar 暴露 ProleCoder view container 和 Chat view。
- 通过 `RpcServerManager.onEvent()` 订阅 live `agent.event`。
- 使用 `ChatEventTimeline` 把 `assistant.delta`、tool lifecycle、approval、context/provider 和 terminal event 转换为 timeline item。
- 同一 run/turn 的连续 `assistant.delta` 会合并为一条 assistant 消息，避免流式输出刷屏。
- 提供 prompt 输入和 mode 选择，通过 Webview `submitTurn` 消息调用 typed `RpcServerManager.sendTurn()`，accepted 后等待同一 run 的 terminal event 收口输入状态。

`vscode/extension/src/commands.ts` 还提供 `requestApproval`：

- 使用 VS Code modal warning 展示审批摘要。
- 将 `Approve` / `Approve Once` / `Approve For Session` / `Reject` / 关闭弹窗映射为稳定的批准或拒绝决定。

`vscode/extension/src/approvalFlow.ts` 当前接入真实 RPC pending approval：

- 订阅 `RpcServerManager.onEvent()`，只处理 `tool.approvalRequired`。
- 校验 approval payload 的 `approvalId`、`toolCallId`、`toolName`、`risk`、`title`、`detail`、`persistable`、`command` 和 `paths`。
- 复用 `requestApproval` 打开 VS Code modal，并把 approve/reject 结果发送为 typed `RpcServerManager.approve()` / `reject()`。
- 记录已处理的 approvalId，避免重复事件触发重复弹窗。

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
2. 稳定共享 RPC 全双工事件管线。已完成：`agent.sendTurn` 早返回、后台持续推送事件，并在断连时取消 active run。
3. 渲染 `agent.event` 事件流。已完成 Sidebar Chat 首版，能消费 manager 转发的事件。
4. 支持文本输入并通过 `agent.sendTurn` 发送真实 turn。已完成首版 Sidebar Chat 输入发送。
5. 通过 JSON-RPC request client 回传用户动作。已完成 approval approve/reject 回传。
6. 展示审批请求和命令输出摘要。已完成首版 `tool.approvalRequired` modal 接入真实 RPC pending queue。
7. 接入命令风险分类器输出，在审批 UI 中展示动态升级后的风险等级和原因。已完成：approval modal 和 Sidebar Chat 时间线都会展示 `riskReasons`。
8. 使用 VS Code 原生 diff editor 展示 patch，并为 hunk 级审批预留交互边界。已完成：`PatchDiffPreviewController` 缓存 `tool.requested` 中的 `apply_patch` unified diff，在审批 modal 前打开虚拟 after 文档与 workspace before 文档的原生 diff，并保存 whole-patch 模式下的稳定 hunk boundary。
9. 展示 Run List / resume。已完成：Sidebar Chat 顶部 Run List 调用 `agent.listRuns` 展示最近 run summary，点击历史 run 后调用 `agent.resume`，清空当前事件视图并消费 replay 的 `agent.event`。
10. 展示 Context Capsule 可视化。已完成：Sidebar Chat 消费 `context.built` metadata，展示三层 token 分布、input/stable budget、cache/estimator 摘要、included/omitted sources 和 manifest 摘要。

Phase 3 P0 验收标准：

- `agent.sendTurn` 创建 run 后返回 accepted，不等待 `assistant.delta`、审批或 terminal event。
- 同一 run 的 live `agent.event` notification 按 Run Log `seq` 顺序输出。
- `agent.resume` 从指定 `replayFromSeq` 回放事件，且回放结果与 live notification 使用相同 envelope。
- stdin EOF、writer BrokenPipe 或插件停用会取消 active run；run log 最终出现 `run.canceled` 或已有 terminal event。
- Sidebar Chat 能消费 `agent.event` 并展示 `assistant.delta`、tool lifecycle 和 terminal event。已完成首版事件渲染。
- Chat 输入能发送真实 `agent.sendTurn`，并通过事件流收到最终结果。已完成首版输入发送和事件流收口。
- `tool.approvalRequired` 触发 VS Code modal，approve/reject 能回传到 `agent.approve` / `agent.reject`。已完成首版真实 RPC pending queue 接入。
- Sidebar Chat 能通过 `agent.listRuns` 展示最近 run，并用 `agent.resume` 回放历史事件。已完成首版 Run List / resume 接入。
- Sidebar Chat 能把 `context.built` 渲染为 Context Capsule 面板，展示 token 分段、来源和 manifest/cache/estimator metadata。已完成首版 Context Capsule 可视化。

Phase 4 P1/P2 深度集成：

1. 读取 Problems 面板诊断并交给 Agent Core。
2. Terminal command approval 展示命令、cwd、风险等级、输出摘要和持久化选项。
3. provider、model、预算、审批策略和 RPC 命令配置界面。
4. FIM completion preview。
5. VSIX alpha / pre-release 打包与安装说明。

在这些能力稳定前，不在插件侧重复实现 context builder、tool execution 或 provider 调用。

## 后续增强

- `agent.sendTurn`、`agent.approve` 和 `agent.reject` 类型化 helper 已完成；继续为 `agent.cancel` 等常用方法增加类型化 helper，避免 UI 层直接拼 method string。
- 支持 `agent.cancel`，在用户关闭 run 或插件停用时取消 pending run。
- 处理协议版本不匹配：显示 server/client protocol version，并引导用户升级对应组件。
- 支持多 workspace folder：每个 workspace root 对应一个 RPC server 或明确选择 active workspace。
- 渲染 `agent.event` 流，包括 assistant delta、计划、工具调用、审批请求、patch 和验证结果。
- 扩展 Native diff editor 当前的 hunk boundary，支持真实 hunk 级选择、部分批准和对应 RPC/Core 协议扩展。
- 从 Problems 面板读取 diagnostics，通过协议传给 Agent Core，而不是在插件内自行生成修复逻辑。
- 增加 `@vscode/test-electron` 集成测试，覆盖真实 extension activation、配置读取、启动失败提示和基础事件渲染。
