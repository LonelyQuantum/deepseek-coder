# Agent Turn Loop

状态：Phase 1 基础编排、TurnProvider async / streaming 边界、真实 DeepSeek 文本 streaming 联网验收、streaming tool call 增量拼装验证、基础 RPC 事件桥接、双向 request loop、真实 RPC Turn Loop handler、`TurnEventSink` 实时事件输出、CLI 交互式审批、RPC pending approval 等待队列、审批超时和取消语义已实现；TUI/VS Code 已有审批交互原语，尚未接入真实 RPC 队列。

Agent Turn Loop 是 Agent Core 的回合编排层。它负责把已经实现的 Context Builder、`reasoning_content` 状态机、provider 边界、工具执行、审批和 Run Log 串成同一条可复现事件流。

## 当前实现位置

```text
crates/agent-core/src/turn_loop.rs
```

当前实现提供：

- `AgentTurnLoop`：持有 provider、审批策略、工具执行器、reasoning 状态机和回合配置。
- `TurnProvider`：异步 streaming provider trait。`complete_stream` 返回 `TurnProviderEvent` 流，provider 可以先发送 `AssistantDelta`，再发送唯一的 `Completed` 响应。
- `TurnProviderEvent`：当前包含 `AssistantDelta` 和 `Completed`。`AssistantDelta` 只用于前端展示和 run log 增量；`Completed` 必须包含完整 content、`reasoning_content` 和 tool calls，供后续 reasoning replay 与工具执行使用。
- `ApprovalPolicy`：审批策略 trait。策略可以批准、拒绝、取消、过期，或返回策略错误；Turn Loop 会把决定写入 `tool.approvalResolved`。
- `RejectAllApprovalPolicy`：默认拒绝所有需要审批的工具，避免写入和命令被静默执行。
- `AutoApprovePolicy`：测试用策略，用于验证已批准工具的执行路径。
- `AgentTurnInput` / `AgentTurnOutcome`：最小 turn 输入与结果。
- `TurnEventSink` / `NoopTurnEventSink`：Run Log 事件追加成功后的实时输出出口。默认 `run_turn` 使用 no-op sink；CLI/RPC 可以调用 `run_turn_with_event_sink` 接入 JSON-RPC notification writer。

## 当前回合流程

```text
AgentTurnInput
  -> ContextBuilder
  -> run log: context.built
  -> ReasoningContentStateMachine
  -> provider.complete_stream
  -> assistant.delta stream
  -> provider Completed response
  -> assistant final 或 assistant tool_calls
  -> tool.requested
  -> tool.approvalRequired when needed
  -> tool.started
  -> WorkspaceToolExecutor
  -> redacted_tool_result_value
  -> tool.completed
  -> append tool result message
  -> next provider.complete
  -> run.completed
```

工具结果写入 run log 或进入下一轮 prompt 前会通过 `redacted_tool_result_value` 转成已脱敏 JSON。原始工具结果仍由工具执行层返回，便于即时诊断和后续 UI 展示，但 Turn Loop 的持久化与模型回传路径使用脱敏结果。

## Run Log 事件

当前基础实现写入以下事件：

- `run.started`
- `turn.started`
- `context.built`
- `provider.requested`
- `assistant.delta`
- `tool.requested`
- `tool.approvalRequired`
- `tool.approvalResolved`
- `tool.started`
- `tool.completed`
- `run.completed`
- `run.failed`

`provider.requested` 只记录消息数量、iteration 和 reasoning replay 状态，不记录完整模型输入。完整上下文由 `context.built` 的 token/source 报告和后续工具结果共同复现；更细的 provider request 摘要应在后续 schema 中设计。

provider stream 中的 content delta 会立即写入 `assistant.delta`，payload 包含 `stream: true`。如果 provider 没有发送 content delta，Turn Loop 会在收到 `Completed` 后把完整 final content 作为一次 `assistant.delta` 写入。Provider-private `reasoning_content` delta 不写入 `assistant.delta`，只由 provider 聚合后放入 `Completed.reasoning_content`，供 `ReasoningContentStateMachine` 校验和下一轮 replay。

`Completed.content` 是最终 assistant 消息文本的权威来源，用于 `run.completed.summary` 或 assistant tool-call replay。`assistant.delta` 是展示和 run log 增量事件，不反向推断最终文本；当 provider 已经发送过可见 content delta 时，Turn Loop 不会在 `Completed` 时重复写一份完整 `assistant.delta`。因此 reasoning delta 不进入用户可见 summary，tool call 前的可见文本如果存在，应由 provider 同时保留在最终 `Completed.content` 中。

DeepSeek streaming tool call delta 在 CLI provider wrapper 内通过 `ChatToolCallAccumulator` 拼装为完整 `ChatToolCall` 后才进入 `Completed.tool_calls`。Turn Loop 不直接处理 provider 私有 delta 形态，只要求 provider 在 `Completed` 中提供完整、可校验、可执行的工具调用列表。

`run.started` 记录规范化后的 workspace root，用于本地审计和前端展示当前 run 绑定的工作区。该路径只应进入本地 run log 和本机前端事件流，不应被上传到公开仓库或远程日志。

## 审批边界

`read_file`、`search`、`git_status` 和 `git_diff` 使用静态 `read` 风险，不需要审批。

`apply_patch` 和 `shell` 当前根据工具定义触发审批：

- 默认策略 `RejectAllApprovalPolicy` 会拒绝执行，Turn Loop 写入 `tool.approvalRequired` 和 `tool.approvalResolved` 后以 `E_APPROVAL_REJECTED` 失败。
- 测试可使用 `AutoApprovePolicy` 验证已批准路径。

命令风险分类器尚未实现。也就是说，`shell` 当前只按静态 `exec` 风险处理，不会识别 `git push`、依赖安装、删除或发布命令并升级为 `network` / `destructive`。该能力应在审批链路接入 CLI/RPC 后实现。

RPC handler 当前提供单 active run 的真实审批等待：当 `tool.approvalRequired` 写入 run log 后，RPC 层会把该请求登记为 pending approval，后台 Turn Loop worker 在 `ApprovalPolicy::decide` 中等待。前端发送 `agent.approve` 会继续进入 `tool.started` 和工具执行；发送 `agent.reject` 会写入拒绝事件并使当前 run 失败；发送 `agent.cancel` 会写入 `tool.approvalResolved(decision="canceled")` 和 `run.canceled`；超过默认 300 秒没有决定会写入 `tool.approvalResolved(decision="expired")` 和 `run.canceled`。这个实现解决了真实审批等待、取消和超时，但还没有全双工事件 writer，也不能强制取消已经进入中的 provider 请求或工具进程。

## 当前测试覆盖

基础 Turn Loop 测试使用 fake provider 覆盖：

- provider 请求 `read_file`，Turn Loop 执行工具、写入 run log、把 tool result message 回传给下一次 provider，并最终完成 run。
- provider 请求 `shell`，默认审批策略拒绝执行，run 失败且不会写入 `tool.started`。
- provider 请求 `apply_patch`，测试审批策略批准后修改文件、记录 `changedFiles`，并完成 run。
- provider 发送多个 streaming content delta，Turn Loop 写入多条 `assistant.delta`，并避免在 `Completed` 时重复写入完整文本。
- Turn Loop 每次成功追加 Run Log 事件后，会把同一条事件交给 `TurnEventSink`，sink 看到的事件序列与本地 `events.jsonl` 一致。
- DeepSeek wrapper 能把 streaming tool call delta 拼成完整工具调用，并在缺少必要 metadata 时失败。

这些测试验证的是模块集成骨架，不需要真实 DeepSeek API Key，也不会联网。真实 tool call delta 形态由 `live_streaming_tool_call_accumulator_smoke_test` 作为手动 opt-in live test 验收。

## 尚未实现

- tool call 参数的 JSON Schema 校验层；当前由具体 Rust 参数类型反序列化保证基础结构。
- 全双工异步 RPC run 执行、独立事件发送队列和取消；当前 `AgentTurnLoopRpcHandler` 已有后台 worker 和审批等待，但事件仍在 request 返回时由 request loop flush。
- 审批过期、多 active run 关联和持久批准存储。
- run summary metadata。
- 命令风险分类器和更强 sandbox。
- RPC request loop 里的 verification 编排；CLI `run` 已支持用户显式 `--verify`。
- `--json` 失败路径的 JSON-RPC error response。

## 后续增强

- 增加真实 CLI 工具调用端到端验收，把 tool call accumulator、审批、工具执行、继续请求和最终输出串成完整闭环。
- 将 `TurnProviderEvent` 扩展到 usage/cache summary，并写入 provider summary 事件。
- 将 `provider.requested`、`tool.completed` 和 `run.completed` payload schema 与 `docs/json-rpc-protocol.md` / `packages/protocol` 对齐。
- 在 Turn Loop 中加入可取消执行模型，确保 provider streaming、工具执行和审批等待都能被 CLI/RPC 中止。
- 将 `ReasoningContentState::ReplayRequired` 的摘要写入更稳定的 run log schema，并关联 tool call id。
- 增加端到端 smoke test，验证 CLI/RPC 从同一份 run log 重建关键过程。
