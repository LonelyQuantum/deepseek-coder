# Agent RPC Server

状态：`0.1.0` Phase 1 基础 stdio 事件桥接和双向 request loop 已实现。

Agent RPC Server 是 CLI、TUI、VS Code 插件和 Rust Agent Core 之间的协议边界。它不重新实现工具执行、上下文构建或 turn loop；它负责把前端 request 转换为 Core 调用，把 Core / Run Log 事件转换为 JSON-RPC notification。

## 设计目标

- 使用 newline-delimited JSON-RPC 2.0 over stdio，便于 CLI、TUI 和 VS Code 复用。
- 事件来源以 Run Log 为准，避免 UI 看到的事件和本地审计日志分叉。
- 事件桥接只做协议封装和时间格式转换，不修改 payload 语义。
- 所有输出都是 UTF-8 单行 JSON，便于前端按行读取和恢复。
- 不把 API Key、环境变量或本机敏感路径写入协议层；Run Log 写入前已做基础脱敏，RPC 层后续还要补充输出截断策略。

## 当前实现范围

当前 `crates/agent-rpc` 实现了最小 stdio 事件桥接和 request loop：

- `RpcMethod`：生成 `agent.initialize`、`agent.event` 等协议方法名。
- `AgentEventEnvelope`：对应 `docs/json-rpc-protocol.md` 中的 server event envelope。
- `JsonRpcRequest<T>` / `JsonRpcResponse<T>` / `JsonRpcErrorResponse`：JSON-RPC 2.0 request/response/error 基础结构。
- `JsonRpcNotification<T>`：JSON-RPC 2.0 notification 基础结构。
- `run_log_event_to_envelope`：把 `RunLogEvent` 转换为前端事件 envelope。
- `run_log_event_to_notification`：把 `RunLogEvent` 转换为 `agent.event` notification。
- `StdioEventBridge<W>`：把一个或多个 Run Log 事件写为 newline-delimited JSON notification。
- `AgentRpcRequestHandler`：RPC request loop 与真实 Core 执行逻辑之间的 handler trait。
- `AgentRpcServer<H>`：维护初始化状态，解析单行 JSON-RPC request，分发给 handler，并写回 response / error。
- `run_stdio_request_loop`：从 `BufRead` 逐行读取 newline-delimited JSON-RPC message，到 EOF 为止。

当前 request loop 已支持 `agent.initialize`、`agent.sendTurn` 和 `agent.resume` 的基础分发。RPC 层本身不直接绑定 DeepSeek provider 或 fixture provider；真实 turn 执行由外部实现 `AgentRpcRequestHandler` 后注入。`agent.approve`、`agent.reject`、`agent.cancel` 和 `agent.listRuns` 仍未接入 handler 分发。

## 数据流

```text
Agent Core / Turn Loop
  -> RunLog.append(...)
  -> RunLogEvent
  -> crates/agent-rpc::run_log_event_to_notification
  -> {"jsonrpc":"2.0","method":"agent.event","params":...}\n
  -> stdout
  -> CLI/TUI/VS Code
```

请求方向的数据流：

```text
stdin line
  -> parse JSON-RPC request
  -> enforce agent.initialize first
  -> deserialize params
  -> AgentRpcRequestHandler
  -> JSON-RPC response
  -> optional agent.event replay
  -> stdout
```

事件 envelope 示例：

```json
{
  "jsonrpc": "2.0",
  "method": "agent.event",
  "params": {
    "seq": 1,
    "time": "1970-01-01T00:00:00.000Z",
    "type": "run.started",
    "runId": "run_01",
    "payload": {
      "mode": "ask"
    }
  }
}
```

## 时间格式

Run Log 内部使用 `timeUnixMs`，RPC 层对外输出 UTC RFC 3339 风格字符串：

```text
YYYY-MM-DDTHH:MM:SS.mmmZ
```

当前实现没有引入额外时间库，而是使用确定性的 UTC civil date 转换。这样能保持 RPC crate 轻量，也避免本地时区影响前端事件排序。

## 与 Run Log 的关系

Run Log 是事实来源：

- `seq` 保持 run 内单调递增。
- `runId` 和 `turnId` 直接来自 `RunLogEvent`。
- `payload` 保持原样传递。
- `timeUnixMs` 只在 RPC 层转换为 `time` 字符串。

RPC 层不负责重新脱敏 payload。当前 Run Log 写入时已经调用基础脱敏规则；后续在真实前端接入前，还需要实现输出大小限制、截断原因和更完整的密钥形态识别。

`agent.sendTurn` 与 `agent.resume` 的 handler 可以返回一组 `RunLogEvent`。request loop 会先写 JSON-RPC response，再按顺序写 `agent.event` notification。这样保持“request 已被接受”和“事件开始抵达”的边界清晰。后续接入真实 Turn Loop 时，实时事件应由同一个 writer 队列持续发送，而不是等 run 完成后批量返回。

## Request Loop 规则

- 空行会被忽略。
- 每行只处理一条 JSON-RPC message，不支持 batch request。
- client notification 没有 `id`，request loop 不返回 response，也不改变初始化状态。
- `agent.initialize` 必须是第一条带 `id` 的 request。
- `agent.initialize` 只能成功一次。
- `protocolVersion` 必须与 `0.1.0` 精确匹配，否则返回 `E_UNSUPPORTED_PROTOCOL`。
- `agent.sendTurn` 和 `agent.resume` 必须在初始化成功后调用。
- handler 返回的 `RunLogEvent` 按原顺序写出为 `agent.event` notification。

基础错误处理已覆盖：

- JSON 解析失败：`-32700`。
- 非 object、版本错误、初始化顺序错误：`-32600`。
- 未支持 method：`-32601`。
- params 反序列化失败：`-32602`。
- 协议不兼容：`-32001`。

## TypeScript 协议

`packages/protocol` 已补充：

- `jsonRpcVersion`
- `agentInitializeMethod`
- `agentSendTurnMethod`
- `agentResumeMethod`
- `agentApproveMethod`
- `agentRejectMethod`
- `agentCancelMethod`
- `agentListRunsMethod`
- `agentEventMethod`
- `JsonRpcRequest<TParams>`
- `JsonRpcResponse<TResult>`
- `JsonRpcErrorResponse<TData>`
- `JsonRpcNotification<TParams>`
- `AgentInitializeParams` / `AgentInitializeResult`
- `SendTurnParams` / `SendTurnResult`
- `ResumeParams` / `ResumeResult`
- `AgentEventEnvelope<TPayload>`
- `AgentEventNotification<TPayload>`

这让 VS Code 插件和未来 TUI/CLI 前端可以用同一套 TypeScript 类型发送 request、处理 response/error，并消费 Rust RPC 事件。

## 后续实现

- 把 CLI 当前的 provider / Turn Loop 选择逻辑抽成可复用 handler，实现真实 `agent.sendTurn`。
- 异步审批等待：把 `tool.approvalRequired` 暂存为 pending approval，等待 `agent.approve` 或 `agent.reject`。
- `agent.resume`：从已有 Run Log 按 `replayFromSeq` 重放事件，目前 request loop 已有 handler 边界，仍需要真实 Run Log handler 实现。
- `agent.listRuns`：依赖 run summary metadata，避免扫描完整 JSONL。
- 事件发送队列：保证同一 run 只有一个 writer 串行发送事件。
- 输出节流：对高频 `assistant.delta` 做批量发送或节流，避免 stdio 前端卡顿。
