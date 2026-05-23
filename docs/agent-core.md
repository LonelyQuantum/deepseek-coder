# 智能体核心（Agent Core）

状态：草案，Phase 1 部分实现。

Agent Core 是 CLI、TUI 和 VS Code 共用的执行引擎。它负责模型回合、上下文构建、工具执行、审批和 run log。

## 职责

- 构建 Context Capsule。
- 调用 provider adapter。
- 校验模型 tool call。
- 对高风险操作发起审批。
- 执行已批准的工具。
- 记录 run log。
- 向前端输出结构化事件。

## reasoning_content 状态机

思考模式下如果 assistant 消息触发 tool calls，后续请求必须能够完整回传该 assistant 消息的 `reasoning_content`。这条规则由 Agent Core 统一实现，详见 `docs/reasoning-content.md`。

Agent Core 在调用 provider 前应先准备消息：

```text
run-log messages
  -> ReasoningContentStateMachine
  -> provider request messages
```

没有 tool calls 的普通 assistant 消息会剥离 `reasoning_content`；带有 tool calls 的 assistant 消息必须保留非空 `reasoning_content`，否则回合显式失败。

## 回合生命周期

```text
user turn
  -> load workspace state
  -> build context capsule
  -> call model
  -> stream assistant deltas and tool calls
  -> validate tool call arguments
  -> request approval when needed
  -> execute tool
  -> append result to run log
  -> continue until final response
  -> run verification when applicable
```

## 失败规则

当必要输入缺失或无效时，Agent Core 应显式失败：

- tool-call JSON 无效
- 必需上下文无法放入预算
- 审批被拒绝
- 命令执行失败且没有明确下一步
- provider 响应不符合预期的流式结构

Agent Core 不应通过启发式后处理掩盖失败。

## Context Builder

Phase 1 已实现基础 Context Builder，详见 `docs/context-capsule.md`。它能从用户任务、项目规则、git 状态、文件、工具结果和计划等片段生成稳定排序的上下文输入，并输出 token 预算报告。

当前 token 统计使用 `utf8_bytes` 估算器，不是 DeepSeek tokenizer 的精确 token 数。Context Builder 已接入基础 Agent Turn Loop，并会写入 `context.built` run log 事件；后续还需要通过 JSON-RPC 发送给前端。

## 本地工具执行

Phase 1 已实现 `WorkspaceToolExecutor`，作为 read/search/apply_patch/shell/git 工具的基础执行层。它负责 workspace 路径解析、敏感路径拒绝、命令超时和结构化工具结果。详细设计见 `docs/tool-system.md`。

当前执行层已接入基础 Agent Turn Loop，可以跑通“模型请求工具 -> 请求审批 -> 执行工具 -> 写入 run log -> 继续下一轮模型调用”的 fake provider 集成测试。尚未完成的是真实 DeepSeek streaming、JSON Schema 校验层、RPC 审批等待和 CLI 对接。

## Run Log

Phase 1 已实现基础 Run Log 存储层，详见 `docs/run-log.md`。它提供 workspace 内 `.deepseek-coder/runs/<runId>/events.jsonl` 追加写入、按序读取、序列校验和基础脱敏。

当前 Run Log 已接入基础 Agent Turn Loop，会记录 user turn、context、provider 请求摘要、工具请求、审批、工具结果和 run 完成/失败事件。`RunLog` 是单 writer 句柄，不提供内部跨任务同步；RPC 层接入时仍应串行化同一个 run 的所有写入。验证命令、run summary 和更完整 provider 摘要仍待实现。

## Agent Turn Loop

Phase 1 已实现基础 Agent Turn Loop，详见 `docs/turn-loop.md`。当前编排层可以用 fake provider 跑通 Context Builder、`ReasoningContentStateMachine`、工具请求、审批、工具执行、脱敏工具结果、Run Log 写入和继续 provider 请求。

当前 Turn Loop 仍是同步 provider trait 与本地库测试骨架，尚未接入真实 DeepSeek streaming、Agent RPC Server 或 CLI。它的价值是先固定模块协作边界和 run log 事件顺序，为后续 RPC/CLI 接入提供可测试核心。

## Phase 1 收敛顺序

当前最重要的目标不是继续扩展工具数量，而是把已有模块串成可运行闭环：

1. Agent RPC Server：把同一条 run log 事件流转换成 JSON-RPC notifications。
2. CLI 最小闭环：通过 `deepseek-coder run "<task>"` 跑通小型仓库上的读取、修改、验证和报告。
3. 端到端 smoke test：使用 fake provider 或 fixture 验证 turn loop、工具执行、run log 和前端事件一致。

## 后续增强

- 将真实 DeepSeek provider streaming 接入 Agent Turn Loop，把 tool call 收集、schema 校验、审批、工具执行和继续请求串成一个可取消的回合。
- 扩展 Run Log 接入范围，保证 provider streaming 摘要、patch、验证命令、取消和恢复都能被本地复现。
- 在调用 provider 前统一运行 `ReasoningContentStateMachine`，确保 thinking + tool calls 的 `reasoning_content` 回放规则不分散到前端。
- 扩展 Context Builder 接入，把 workspace manifest、git 状态、选中文件、工具结果和计划步骤纳入 provider 请求。
- 在工具结果进入 run log 或下一轮 prompt 前增加统一脱敏与大小限制。
- 把 Agent Core 事件通过 `crates/agent-rpc` 映射到 `docs/json-rpc-protocol.md`，供 CLI/TUI/VS Code 共享。
