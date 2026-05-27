# reasoning_content 状态机

状态：已实现，Phase 1。

本文档定义 Agent Core 如何处理 DeepSeek 思考模式下的 `reasoning_content`。provider adapter 只负责序列化和反序列化字段；是否在下一次请求中回传、是否剥离、是否报错，由 Agent Core 的状态机统一决定。

## 目标

- 在发生 tool calls 时，保留对应 assistant 消息的完整 `reasoning_content`，用于后续请求回传。
- 没有 tool calls 的普通 assistant 消息不回传 `reasoning_content`，减少 token 消耗和上下文噪声。
- 输入不满足协议时显式失败，不用空字符串、猜测或后处理修正。
- 让 CLI、TUI、VS Code 插件共用同一套规则，避免各前端行为分叉。

## 实现位置

```text
crates/agent-core/src/reasoning.rs
```

公开类型：

- `ReasoningContentMode`：当前请求是否启用 thinking。
- `ReasoningContentStateMachine`：根据 mode 准备下一次 provider 请求消息。
- `ReasoningContentState`：准备后的 replay 状态。
- `PreparedReasoningMessages`：准备后的消息和状态快照。
- `ReasoningContentError`：协议错误。

## 状态

```text
NoReplayRequired
ReplayRequired { assistant_messages }
```

`NoReplayRequired` 表示当前消息列表中没有必须回传 `reasoning_content` 的 assistant tool-call 消息。

`ReplayRequired` 表示至少一个 assistant tool-call 消息必须保留 `reasoning_content`。`assistant_messages` 记录这类消息数量，便于后续 run log、诊断和 UI 展示。

## 规则

### thinking enabled

- 非 assistant 消息不得包含 `reasoning_content`，否则返回 `UnexpectedReasoningContentRole`。
- assistant 消息没有 `tool_calls` 时，状态机会移除 `reasoning_content`。
- assistant 消息有非空 `tool_calls` 时，必须有非空 `reasoning_content`，否则返回 `MissingRequiredReasoningContent`。
- assistant tool-call 消息的 `reasoning_content` 原样保留，不做摘要、不做裁剪、不做拼接。
- 通过 `ChatMessage::assistant_with_tool_calls` 构造回放消息时，即使模型返回的 `content` 为空，也会序列化为空字符串，保证 tool-call assistant 消息始终包含稳定的 `content` 字段。

### thinking disabled

- 非 assistant 消息仍不得包含 `reasoning_content`。
- assistant 消息的 `reasoning_content` 全部移除。
- 即使 assistant 消息带有 `tool_calls`，也不会产生 replay requirement。

## 请求准备流程

```text
stored/run-log messages
  -> ReasoningContentStateMachine::prepare_messages
  -> provider request messages
  -> DeepSeek API adapter
```

状态机只处理消息字段，不调用网络、不执行工具、不修改 run log。后续 Agent Turn Loop 应在每次调用 provider 前先通过该状态机准备消息。

## 测试覆盖

- 普通 assistant 消息会剥离 `reasoning_content`。
- assistant tool-call 消息会保留 `reasoning_content`。
- 多轮对话中只保留 tool-call assistant 的 `reasoning_content`。
- thinking enabled 时，assistant tool-call 消息缺少 `reasoning_content` 会显式失败。
- thinking disabled 时，assistant 的 `reasoning_content` 会全部剥离。
- 非 assistant 消息带有 `reasoning_content` 会显式失败。

## 真实联网测试

真实联网测试位于：

```text
crates/agent-core/tests/deepseek_api_live.rs
```

`live_reasoning_content_tool_replay_smoke_test` 会执行两次短请求：

1. 启用 thinking，只提供 `get_live_reasoning_fixture` 这一个工具，并提示模型调用它。
2. 通过 `ReasoningContentStateMachine` 准备消息，回传 assistant tool-call 消息的 `reasoning_content` 和 tool result，再请求最终回答。

该测试用于验证本地状态机准备出的消息确实能被 DeepSeek API 接受。它比基础 smoke test 多一次请求，并启用 thinking，因此只建议手动单独运行：

```powershell
$env:PROLE_CODER_LIVE_TESTS = "1"
$env:DEEPSEEK_BASE_URL = "https://api.deepseek.com"
$env:DEEPSEEK_MODEL = "deepseek-v4-flash"
cargo test -p prole-coder-agent-core --test deepseek_api_live live_reasoning_content_tool_replay_smoke_test -- --ignored --exact --nocapture
```

测试将第一轮 `max_tokens` 控制为 256，第二轮控制为 128。不要把它加入默认 CI。

DeepSeek V4 thinking mode 不应发送 `tool_choice`，因此该测试不使用 `tool_choice` 强制调用工具。

## 后续增强

- 接入 Agent Turn Loop，在每次 provider 请求前自动准备消息，避免 CLI/TUI/VS Code 分别处理 `reasoning_content`。
- 在 run log 中记录 replay 状态、安全摘要和关联 tool call id，但默认不把完整 provider-private reasoning 展示给 UI。
- 增加多 assistant tool-call、多工具结果、工具失败后继续请求和取消回合的测试用例。
- 增加跨 provider 能力判断：只有 provider 声明需要回传私有 reasoning 时才启用该状态机的严格规则。
- 与上下文预算系统衔接，明确 `reasoning_content` 回放占用的 token 预算，并在超预算时显式失败。
