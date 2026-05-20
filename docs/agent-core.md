# 智能体核心（Agent Core）

状态：草案。

Agent Core 是 CLI、TUI 和 VS Code 共用的执行引擎。它负责模型回合、上下文构建、工具执行、审批和 run log。

## 职责

- 构建 Context Capsule。
- 调用 provider adapter。
- 校验模型 tool call。
- 对高风险操作发起审批。
- 执行已批准的工具。
- 记录 run log。
- 向前端输出结构化事件。

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
