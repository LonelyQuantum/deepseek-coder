# Agent Turn Loop

状态：Phase 1 基础编排已实现，真实 provider streaming 和前端 RPC 接入尚未实现。

Agent Turn Loop 是 Agent Core 的回合编排层。它负责把已经实现的 Context Builder、`reasoning_content` 状态机、provider 边界、工具执行、审批和 Run Log 串成同一条可复现事件流。

## 当前实现位置

```text
crates/agent-core/src/turn_loop.rs
```

当前实现提供：

- `AgentTurnLoop`：持有 provider、审批策略、工具执行器、reasoning 状态机和回合配置。
- `TurnProvider`：同步 provider trait，当前用于 fake provider / fixture 测试；真实 DeepSeek streaming 适配将在后续接入。
- `ApprovalPolicy`：审批策略 trait。
- `RejectAllApprovalPolicy`：默认拒绝所有需要审批的工具，避免写入和命令被静默执行。
- `AutoApprovePolicy`：测试用策略，用于验证已批准工具的执行路径。
- `AgentTurnInput` / `AgentTurnOutcome`：最小 turn 输入与结果。

## 当前回合流程

```text
AgentTurnInput
  -> ContextBuilder
  -> run log: context.built
  -> ReasoningContentStateMachine
  -> provider.complete
  -> assistant.delta 或 assistant tool_calls
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
- `tool.started`
- `tool.completed`
- `run.completed`
- `run.failed`

`provider.requested` 只记录消息数量、iteration 和 reasoning replay 状态，不记录完整模型输入。完整上下文由 `context.built` 的 token/source 报告和后续工具结果共同复现；更细的 provider request 摘要应在后续 schema 中设计。

## 审批边界

`read_file`、`search`、`git_status` 和 `git_diff` 使用静态 `read` 风险，不需要审批。

`apply_patch` 和 `shell` 当前根据工具定义触发审批：

- 默认策略 `RejectAllApprovalPolicy` 会拒绝执行，Turn Loop 写入 `tool.approvalRequired` 后以 `E_APPROVAL_REJECTED` 失败。
- 测试可使用 `AutoApprovePolicy` 验证已批准路径。

命令风险分类器尚未实现。也就是说，`shell` 当前只按静态 `exec` 风险处理，不会识别 `git push`、依赖安装、删除或发布命令并升级为 `network` / `destructive`。该能力应在审批链路接入 CLI/RPC 后实现。

## 当前测试覆盖

基础 Turn Loop 测试使用 fake provider 覆盖：

- provider 请求 `read_file`，Turn Loop 执行工具、写入 run log、把 tool result message 回传给下一次 provider，并最终完成 run。
- provider 请求 `shell`，默认审批策略拒绝执行，run 失败且不会写入 `tool.started`。
- provider 请求 `apply_patch`，测试审批策略批准后修改文件、记录 `changedFiles`，并完成 run。

这些测试验证的是模块集成骨架，不需要真实 DeepSeek API Key，也不会联网。

## 尚未实现

- 真实 DeepSeek adapter 接入 `TurnProvider`。
- streaming delta 到 `assistant.delta` 的逐块事件映射。
- tool call 参数的 JSON Schema 校验层；当前由具体 Rust 参数类型反序列化保证基础结构。
- Agent RPC Server 对 approval request 的异步等待和取消。
- CLI `run` 最小闭环。
- run summary metadata。
- 命令风险分类器和更强 sandbox。
- verification command 编排。

## 后续增强

- 用真实 DeepSeek provider 适配 `TurnProvider`，并保留 fake provider fixture 作为确定性集成测试。
- 将 `provider.requested`、`tool.completed` 和 `run.completed` payload schema 与 `docs/json-rpc-protocol.md` / `packages/protocol` 对齐。
- 在 Turn Loop 中加入可取消执行模型，确保 provider streaming、工具执行和审批等待都能被 CLI/RPC 中止。
- 将 `ReasoningContentState::ReplayRequired` 的摘要写入更稳定的 run log schema，并关联 tool call id。
- 增加端到端 smoke test，验证 CLI/RPC 从同一份 run log 重建关键过程。
