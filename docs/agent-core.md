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

## 本地工具执行

Phase 1 已实现 `WorkspaceToolExecutor`，作为 read/search/apply_patch/shell/git 工具的基础执行层。它负责 workspace 路径解析、敏感路径拒绝、命令超时和结构化工具结果。详细设计见 `docs/tool-system.md`。

当前执行层还是独立库能力，尚未接入完整 Agent 回合。也就是说，工具函数已经能被测试直接调用，但“模型请求工具 -> 校验参数 -> 请求审批 -> 执行工具 -> 写入 run log -> 继续下一轮模型调用”的编排还没有完成。

## Run Log

Phase 1 已实现基础 Run Log 存储层，详见 `docs/run-log.md`。它提供 workspace 内 `.deepseek-coder/runs/<runId>/events.jsonl` 追加写入、按序读取、序列校验和基础脱敏。

当前 Run Log 仍是独立库能力，尚未接入完整 Agent 回合。Agent Turn Loop 实现后，应把 user turn、provider 摘要、工具请求、审批、工具结果、patch 和验证命令都写入同一条事件流。

## Phase 1 收敛顺序

当前最重要的目标不是继续扩展工具数量，而是把已有模块串成可运行闭环：

1. 基础 Context Builder：从用户任务、项目规则、git 状态、选中文件和工具结果生成可审计输入包。
2. Agent Turn Loop：调用 provider、收集 tool calls、校验 schema、请求审批、执行工具、写入 run log，并按工具结果继续下一次请求。
3. Agent RPC Server：把同一条 run log 事件流转换成 JSON-RPC notifications。
4. CLI 最小闭环：通过 `deepseek-coder run "<task>"` 跑通小型仓库上的读取、修改、验证和报告。
5. 端到端 smoke test：使用 fake provider 或 fixture 验证 turn loop、工具执行、run log 和前端事件一致。

## 后续增强

- 实现 Agent Turn Loop，把 provider streaming、tool call 收集、schema 校验、审批、工具执行和继续请求串成一个可取消的回合。
- 将基础 Run Log 接入 Agent Turn Loop，并保证模型输入摘要、工具请求、审批、工具结果、patch 和验证命令都能被本地复现。
- 在调用 provider 前统一运行 `ReasoningContentStateMachine`，确保 thinking + tool calls 的 `reasoning_content` 回放规则不分散到前端。
- 接入 Context Capsule 构建器，把 workspace manifest、git 状态、选中文件、工具结果和计划步骤纳入 token 预算。
- 在工具结果进入 run log 或下一轮 prompt 前增加统一脱敏与大小限制。
- 把 Agent Core 事件通过 `crates/agent-rpc` 映射到 `docs/json-rpc-protocol.md`，供 CLI/TUI/VS Code 共享。
