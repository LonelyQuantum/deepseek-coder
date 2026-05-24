# Agent RPC Server

状态：`0.1.0` Phase 1 基础 stdio 事件桥接、`TurnEventSink` 实时输出桥接、双向 request loop、真实 Turn Loop handler、RPC pending approval 等待队列、审批超时、pending run 取消语义和 Run Log 写入串行化已实现。

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
- `StdioEventBridge<W>`：把一个或多个 Run Log 事件写为 newline-delimited JSON notification，并实现 `TurnEventSink`，可直接接到 `AgentTurnLoop::run_turn_with_event_sink`。
- `AgentRpcRequestHandler`：RPC request loop 与真实 Core 执行逻辑之间的 handler trait。
- `AgentTurnLoopRpcHandler<F>`：通过 provider factory 复用 Core `AgentTurnLoop` 的真实 handler。它会在 `agent.sendTurn` 时创建 run log、启动后台 Turn Loop worker，并把事件返回给 request loop 输出。
- `RpcApprovalQueue` / `RpcApprovalPolicy`：在 `tool.approvalRequired` 事件出现时登记 pending approval，并让后台 Turn Loop 在 `ApprovalPolicy::decide` 中等待 `agent.approve` / `agent.reject` / `agent.cancel` 或超时唤醒。
- `SerializedRunLog`：RPC active run 持有共享的同步 run log，worker append 和 `agent.resume` load 通过同一把锁串行化。
- `AgentRpcServer<H>`：维护初始化状态，解析单行 JSON-RPC request，分发给 handler，并写回 response / error。
- `run_stdio_request_loop`：从 `BufRead` 逐行读取 newline-delimited JSON-RPC message，到 EOF 为止。

当前 request loop 已支持 `agent.initialize`、`agent.sendTurn`、`agent.approve`、`agent.reject`、`agent.cancel` 和 `agent.resume` 的基础分发。`AgentTurnLoopRpcHandler` 已实现真实 `agent.sendTurn`、基于 Run Log 的 `agent.resume`、单 active run 的 pending approval 等待队列、pending approval 超时和取消。RPC crate 本身仍不直接绑定 DeepSeek provider 或 fixture provider；具体 provider 由外部 factory 注入，CLI 的 `rpc` 子命令当前提供 DeepSeek / fixture factory。`agent.listRuns` 仍未实现。

本机可通过以下命令启动 stdio RPC server：

```powershell
deepseek-coder rpc
```

测试和前端开发时可使用 `deepseek-coder rpc --provider fixture --fixture final` 获得不联网的确定性 provider。

VS Code 插件当前已提供基础进程监管：插件激活后会按 `deepseek-coder.rpc.command` 和 `deepseek-coder.rpc.args` 启动该 stdio server，发送 `agent.initialize`，把 stdout 中的 `agent.event` notification 转发给前端事件 handler，并在进程退出或启动失败时更新状态和提示用户。

## 数据流

```text
Agent Core / Turn Loop
  -> RunLog.append(...) / append 后触发 TurnEventSink
  -> RunLogEvent
  -> crates/agent-rpc::StdioEventBridge
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

`agent.sendTurn` 与 `agent.resume` 的 handler 可以返回一组 `RunLogEvent`。request loop 会先写 JSON-RPC response，再按顺序写 `agent.event` notification。这样保持“request 已被接受”和“事件开始抵达”的边界清晰。

当前 `AgentTurnLoopRpcHandler` 仍以 request 为边界返回事件，但已经不再用拒绝策略模拟审批。`agent.sendTurn` 会启动后台 Turn Loop worker，并收集事件直到 run 结束或出现 `tool.approvalRequired`；如果需要审批，response 后会输出审批请求事件，worker 在内存队列中等待。随后 `agent.approve` / `agent.reject` 会解析对应 `approvalId`、唤醒 worker，并继续收集 `tool.approvalResolved`、工具执行和 run 结束事件。`agent.cancel` 可取消正在等待审批的 active run，写入 `tool.approvalResolved(decision="canceled")` 和 `run.canceled`；默认 300 秒审批超时会写入 `tool.approvalResolved(decision="expired")` 和 `run.canceled`。

同一个 active run 的 Run Log 由 `SerializedRunLog` 保护：后台 Turn Loop worker 是唯一实际追加者，`agent.resume` 如果读取的是当前 active run，会通过同一个同步句柄 load，而不是直接绕过锁读取磁盘文件。这样能保证前端 replay 看到的 `seq` 总是来自完整事件边界。

这意味着 Phase 1 已经具备真实 RPC 审批等待、取消和超时语义，但还不是完全全双工的后台 streaming server：长时间 provider 请求仍会占用当前 `agent.sendTurn` request，事件也由 handler 返回给 request loop 后按顺序写出。后续需要加入独立事件 writer 队列、provider/tool 取消信号和 reader/writer 并发编排，让 `agent.sendTurn` 可以更早返回 accepted，并在没有后续 request 的情况下持续推送事件。

## Request Loop 规则

- 空行会被忽略。
- 每行只处理一条 JSON-RPC message，不支持 batch request。
- client notification 没有 `id`，request loop 不返回 response，也不改变初始化状态。
- `agent.initialize` 必须是第一条带 `id` 的 request。
- `agent.initialize` 只能成功一次。
- `protocolVersion` 必须与 `0.1.0` 精确匹配，否则返回 `E_UNSUPPORTED_PROTOCOL`。
- `agent.sendTurn`、`agent.approve`、`agent.reject`、`agent.cancel` 和 `agent.resume` 必须在初始化成功后调用。
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
- `ApproveParams` / `ApproveResult`
- `RejectParams` / `RejectResult`
- `CancelParams` / `CancelResult`
- `ResumeParams` / `ResumeResult`
- `AgentEventEnvelope<TPayload>`
- `AgentEventNotification<TPayload>`

这让 VS Code 插件和未来 TUI/CLI 前端可以用同一套 TypeScript 类型发送 request、处理 response/error，并消费 Rust RPC 事件。

## 后续实现

- 全双工异步 run 执行：让 `agent.sendTurn` 先返回 accepted，再由后台任务持续写出 `agent.event`，而不是只在 request 返回时 flush 事件。
- provider/tool 取消信号：当前 `agent.cancel` 已能取消 pending approval；后续还要取消正在进行中的 provider request 和工具进程。
- `agent.listRuns`：依赖 run summary metadata，避免扫描完整 JSONL。
- 事件发送队列：在异步 RPC Turn Loop handler 中保证同一 run 只有一个 notification writer 串行发送事件。
- 输出节流：对高频 `assistant.delta` 做批量发送或节流，避免 stdio 前端卡顿。
