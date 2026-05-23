# Agent RPC Server

状态：`0.1.0` Phase 1 基础 stdio 事件桥接已实现。

Agent RPC Server 是 CLI、TUI、VS Code 插件和 Rust Agent Core 之间的协议边界。它不重新实现工具执行、上下文构建或 turn loop；它负责把前端 request 转换为 Core 调用，把 Core / Run Log 事件转换为 JSON-RPC notification。

## 设计目标

- 使用 newline-delimited JSON-RPC 2.0 over stdio，便于 CLI、TUI 和 VS Code 复用。
- 事件来源以 Run Log 为准，避免 UI 看到的事件和本地审计日志分叉。
- 事件桥接只做协议封装和时间格式转换，不修改 payload 语义。
- 所有输出都是 UTF-8 单行 JSON，便于前端按行读取和恢复。
- 不把 API Key、环境变量或本机敏感路径写入协议层；Run Log 写入前已做基础脱敏，RPC 层后续还要补充输出截断策略。

## 当前实现范围

当前 `crates/agent-rpc` 实现了最小 stdio 事件桥接：

- `RpcMethod`：生成 `agent.initialize`、`agent.event` 等协议方法名。
- `AgentEventEnvelope`：对应 `docs/json-rpc-protocol.md` 中的 server event envelope。
- `JsonRpcNotification<T>`：JSON-RPC 2.0 notification 基础结构。
- `run_log_event_to_envelope`：把 `RunLogEvent` 转换为前端事件 envelope。
- `run_log_event_to_notification`：把 `RunLogEvent` 转换为 `agent.event` notification。
- `StdioEventBridge<W>`：把一个或多个 Run Log 事件写为 newline-delimited JSON notification。

当前尚未实现完整 request loop，也不会在 RPC 层直接启动真实模型回合。`agent.sendTurn`、`agent.approve`、`agent.reject`、`agent.cancel`、`agent.resume` 和 `agent.listRuns` 的处理会在后续步骤接入。

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

## TypeScript 协议

`packages/protocol` 已补充：

- `jsonRpcVersion`
- `agentInitializeMethod`
- `agentEventMethod`
- `JsonRpcNotification<TParams>`
- `AgentEventEnvelope<TPayload>`
- `AgentEventNotification<TPayload>`

这让 VS Code 插件和未来 TUI/CLI 前端可以用同一套 TypeScript 类型消费 Rust RPC 事件。

## 后续实现

- `agent.initialize`：校验协议版本、workspace trust、workspace root，并返回 server capabilities。
- `agent.sendTurn`：创建或打开 run，启动 Agent Turn Loop，并持续发送 `agent.event`。
- 异步审批等待：把 `tool.approvalRequired` 暂存为 pending approval，等待 `agent.approve` 或 `agent.reject`。
- `agent.resume`：从已有 Run Log 按 `replayFromSeq` 重放事件。
- `agent.listRuns`：依赖 run summary metadata，避免扫描完整 JSONL。
- 事件发送队列：保证同一 run 只有一个 writer 串行发送事件。
- 输出节流：对高频 `assistant.delta` 做批量发送或节流，避免 stdio 前端卡顿。
