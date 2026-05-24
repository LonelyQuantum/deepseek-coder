# 运行日志（Run Log）

状态：Phase 1 基础存储层和写入串行化已实现，并已接入基础 Agent Turn Loop、CLI `run` 和 RPC Turn Loop handler。

Run Log 是 Agent Core 的本地审计记录。它记录一次 run 中发生的事件，使 CLI、TUI、VS Code 和后续调试工具能够读取同一份事实来源。Run Log 不等同于模型上下文；进入上下文前仍需要 Context Capsule 做筛选、摘要、脱敏和 token 预算。

## 目标

- 追加写入，不重写历史事件。
- 每条事件有单调递增的 `seq`。
- 事件可以按原顺序读取，用于 resume、回放和调试。
- 写入前执行基础敏感信息脱敏。
- 日志路径固定在 workspace 内，不依赖 shell 当前目录。

## 存储位置

默认位置：

```text
<workspace>/.deepseek-coder/runs/<runId>/events.jsonl
```

`.deepseek-coder/` 已在 `.gitignore` 中排除，run log 不应进入 Git 仓库。

## Rust 实现

实现位置：

```text
crates/agent-core/src/run_log.rs
```

核心类型：

- `RunLogStore`：绑定 workspace root 和 state dir，负责创建、打开和读取 run。
- `RunLog`：单 writer 追加句柄，维护下一条 `seq`。
- `RunLogWriter`：Turn Loop 使用的写入 trait，让单 writer 和同步 writer 共享同一套回合编排。
- `SerializedRunLog`：`Mutex<RunLog>` 包装，用于跨线程或前端 request 边界串行化同一 run 的 append/load。
- `RunLogEvent`：JSONL 中的一条事件。
- `RunLogError`：路径、序列、JSON、I/O 和标识符错误。

## 写入并发边界

`RunLog` 仍是最小单 writer 句柄，适合 CLI `run` 这种同步流程。它通过 `&mut self` 保证同一代码路径不能同时追加两条事件。

`SerializedRunLog` 用于 RPC 等跨线程场景。它把同一个 `RunLog` 放入 `Mutex`，所有 clone 共享同一个 `next_seq` 和文件句柄状态；每次 append 都先拿锁，写入完成并推进 `seq` 后释放。`load` 也走同一把锁，避免 active run 正在写入时，`agent.resume` 从磁盘读到半条事件或不一致的序列。

当前策略不是全双工事件发送队列：它只保证 run log 本身的 append/load 串行化。RPC stdout 的持续事件推送仍由 request loop flush；后续全双工 server 需要再引入独立事件 writer 队列，保证同一 run 的通知发送也按 `seq` 串行。

## 事件格式

当前内部事件使用 JSONL，每行一条 JSON：

```json
{
  "seq": 1,
  "timeUnixMs": 1770000000000,
  "type": "run.started",
  "runId": "run_01",
  "turnId": "turn_01",
  "payload": {}
}
```

说明：

- `seq` 从 1 开始，读取时要求连续；发现缺口或乱序会显式失败。
- `timeUnixMs` 是 UNIX epoch 毫秒。`crates/agent-rpc` 在转换 JSON-RPC 事件时生成 UTC `time` 字符串。
- `type` 使用 `docs/json-rpc-protocol.md` 中的事件名，例如 `run.started`、`assistant.delta`、`tool.completed`。
- `payload` 当前是 `serde_json::Value`，具体 schema 后续会和 JSON-RPC 协议、TypeScript 协议包对齐。

## 路径与标识符规则

- workspace root 必须是已经存在的目录。
- state dir 必须是 workspace-relative path；绝对路径和 `..` 会失败。
- `runId` 和 `turnId` 只能包含 ASCII 字母、数字、`_` 和 `-`。
- event type 只能包含 ASCII 字母、数字、`.`、`_` 和 `-`。

## 脱敏规则

写入事件前，Run Log 会递归处理 `payload`：

- 以下字段名会整体替换为 `<redacted>`：`apiKey`、`authorization`、`password`、`secret`、`token`、`accessToken`、`refreshToken`、`credential`、`privateKey` 等。
- 字符串中的明显 `sk-...` 形态密钥片段会替换为 `<redacted>`。
- 非敏感统计字段不会因为包含 `Tokens` 后缀而被误删，例如 `cacheHitTokens`。

这只是基础保护层。工具输出进入 prompt 或长期审计包前，仍需要更完整的统一脱敏层。

## 当前测试覆盖

- 追加事件并按 `seq` 读取。
- 重新打开 run 后从正确的下一条 `seq` 继续。
- 拒绝不安全的 run id 和 state dir。
- 读取时发现序列缺口会失败。
- 写入前脱敏敏感字段和明显密钥片段。
- `SerializedRunLog` 多线程 clone 并发追加时，仍生成连续 `seq`，并可被重新打开为正确的下一条序号。

## 后续增强

- 扩展 Agent Turn Loop 接入，自动记录 provider streaming 摘要、patch proposal、验证命令、取消和恢复事件。
- 增加 run summary / metadata 文件，支持 `agent.listRuns` 快速读取标题、状态、开始时间、结束时间和事件计数，避免每次列出 run 都扫描完整 JSONL。
- 增加事件 payload 的强类型 schema，并与 `docs/json-rpc-protocol.md` 和 `packages/protocol` 做兼容性测试。
- 增加日志轮转或分片策略，防止长时间运行和高频 streaming 事件让单个 `events.jsonl` 过大。
- 增加输出截断信息，区分“字段不存在”“输出为空”和“输出因安全或大小限制被截断”。
- 增加 run export 审计包，导出前再次做敏感信息扫描。
