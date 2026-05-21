# JSON-RPC 协议

状态：`0.1.0` 协议版本的已接受草案。

本文档定义 `deepseek-coder` 前端与 Rust Agent RPC Server 之间的内部协议。它不是 DeepSeek API 协议。

## 位置

```text
VS Code / TUI / CLI
        |
        | JSON-RPC 2.0 over stdio
        v
Agent RPC Server
        |
        v
Agent Core
```

该协议让前端保持轻量。前端负责渲染状态、收集用户输入、响应审批提示。Agent Core 负责 turn loop、context construction、provider call、tool execution、approval policy 和 run log。

## 传输

初始传输方式是基于 stdio 的 newline-delimited JSON-RPC 2.0：

- UTF-8 JSON。
- 每行一条 JSON-RPC 消息。
- request id 使用字符串。
- 前端向 server 发送 request。
- server 向前端发送 response 和 event notification。

服务端事件使用 JSON-RPC notification，method 固定为 `agent.event`：

```json
{
  "jsonrpc": "2.0",
  "method": "agent.event",
  "params": {
    "seq": 1,
    "time": "2026-05-20T14:00:00Z",
    "type": "run.started",
    "runId": "run_01",
    "payload": {}
  }
}
```

## 版本

协议版本为 `0.1.0`。

在 `0.x` 阶段，client 和 server 默认要求精确版本匹配；除非双方在 `agent.initialize` 中显式声明兼容版本。

版本变化规则：

- Patch：只做文档澄清。
- Minor：只新增字段、方法或事件。
- Major 或 pre-1.0 incompatible bump：重命名字段、删除字段、改变语义或改变必需行为。

## 标识符

所有 id 都是不透明字符串：

- `runId`：一次 agent run。
- `turnId`：run 内的一次用户 turn。
- `event seq`：run log 内单调递增的整数。
- `approvalId`：一次审批请求。
- `toolCallId`：一次 tool call。
- `patchId`：一次拟议 patch。
- `verificationId`：一次验证命令。

前端不得从 id 前缀推断语义。

## 路径

- `workspaceRoot` 使用绝对平台路径。
- 事件中的文件路径使用 workspace-relative path，并用 `/` 作为分隔符。
- 工具执行必须把路径解析在 workspace 内；除非工具显式声明例外，并获得审批。

## 通用类型

### ClientInfo

```ts
type FrontendKind = "cli" | "tui" | "vscode";

interface ClientInfo {
  name: string;
  version: string;
  frontend: FrontendKind;
}
```

### Capability Set

```ts
interface ServerCapabilities {
  protocolVersion: "0.1.0";
  supportsRunResume: boolean;
  supportsPatchApproval: boolean;
  supportsPersistentApprovals: boolean;
  supportedRiskLevels: RiskLevel[];
}
```

### RiskLevel

```ts
type RiskLevel = "read" | "write" | "exec" | "network" | "destructive";
```

### PlanStep

```ts
type PlanStepStatus = "pending" | "in_progress" | "completed" | "failed" | "canceled";

interface PlanStep {
  id: string;
  title: string;
  status: PlanStepStatus;
  detail?: string;
}
```

## 方法

### `agent.initialize`

连接到 workspace 并协商能力。

Request：

```json
{
  "jsonrpc": "2.0",
  "id": "req_1",
  "method": "agent.initialize",
  "params": {
    "protocolVersion": "0.1.0",
    "client": {
      "name": "deepseek-coder-vscode",
      "version": "0.1.0",
      "frontend": "vscode"
    },
    "workspaceRoot": "C:/workspace/deepseek-coder",
    "workspaceTrusted": true
  }
}
```

Result：

```json
{
  "protocolVersion": "0.1.0",
  "server": {
    "name": "deepseek-coder-agent-rpc",
    "version": "0.1.0"
  },
  "capabilities": {
    "protocolVersion": "0.1.0",
    "supportsRunResume": true,
    "supportsPatchApproval": true,
    "supportsPersistentApprovals": false,
    "supportedRiskLevels": ["read", "write", "exec", "network", "destructive"]
  },
  "stateDir": ".deepseek-coder"
}
```

规则：

- `agent.initialize` 必须是第一条 request。
- 如果 `workspaceTrusted` 为 false，则禁用 write、exec、network 和 destructive 工具。
- 协议不匹配时返回 `E_UNSUPPORTED_PROTOCOL`。

### `agent.sendTurn`

发送用户请求。如果省略 `runId`，server 创建新的 run。

Request params：

```ts
interface SendTurnParams {
  runId?: string;
  message: string;
  mode: "plan" | "edit" | "review" | "ask";
  attachments?: TurnAttachment[];
}

interface TurnAttachment {
  kind: "file" | "selection" | "diagnostic";
  path?: string;
  range?: {
    startLine: number;
    startColumn: number;
    endLine: number;
    endColumn: number;
  };
  text?: string;
}
```

Result：

```ts
interface SendTurnResult {
  runId: string;
  turnId: string;
  accepted: true;
}
```

Result 返回后，进度通过 `agent.event` notification 持续到达。

### `agent.approve`

批准一个 pending approval request。

```ts
interface ApproveParams {
  approvalId: string;
  persist?: "never" | "session" | "workspace";
}
```

规则：

- `persist` 默认是 `never`。
- 只有明确标记为 persistable 的审批类型才能使用 `workspace` 持久化。
- 批准已过期或未知审批时返回 `E_APPROVAL_NOT_FOUND`。

### `agent.reject`

拒绝一个 pending approval request。

```ts
interface RejectParams {
  approvalId: string;
  reason?: string;
}
```

规则：

- Agent Core 将拒绝记录到 run log。
- Agent Core 不得用等价操作绕过拒绝。
- 拒绝后，Agent Core 要么请求新路径，要么继续只读工作，要么停止 run。

### `agent.cancel`

取消 active run。

```ts
interface CancelParams {
  runId: string;
  reason?: string;
}
```

取消完成后，server 发送 `run.canceled`。

### `agent.resume`

恢复或回放之前的 run。

```ts
interface ResumeParams {
  runId: string;
  replayFromSeq?: number;
}
```

Result：

```ts
interface ResumeResult {
  runId: string;
  nextSeq: number;
  replayStarted: boolean;
}
```

规则：

- 如果提供 `replayFromSeq`，server 从该 seq 重新发送事件。
- 如果本地 run log 不存在，返回 `E_RUN_NOT_FOUND`。

### `agent.listRuns`

列出当前 workspace 已知的本地 run。

```ts
interface RunSummary {
  runId: string;
  title: string;
  status: "running" | "completed" | "failed" | "canceled";
  startedAt: string;
  completedAt?: string;
}
```

## 事件封装

所有 server event 使用该 envelope：

```ts
interface AgentEventEnvelope<TPayload> {
  seq: number;
  time: string;
  type: string;
  runId?: string;
  turnId?: string;
  payload: TPayload;
}
```

规则：

- `seq` 在 run log 内单调递增。
- 属于某个 run 的事件必须包含 `runId`。
- 属于某个 turn 的事件应包含 `turnId`。
- 事件必须能在不依赖前端本地状态的情况下追加到 run log。

## 事件

### `run.started`

```ts
interface RunStarted {
  runId: string;
  workspaceRoot: string;
  mode: "plan" | "edit" | "review" | "ask";
}
```

### `run.completed`

```ts
interface RunCompleted {
  summary: string;
  changedFiles: string[];
  verificationStatus: "passed" | "failed" | "skipped";
}
```

### `run.failed`

```ts
interface RunFailed {
  code: string;
  message: string;
  details?: unknown;
}
```

### `run.canceled`

```ts
interface RunCanceled {
  reason?: string;
}
```

### `assistant.delta`

面向用户展示的 assistant 输出。

```ts
interface AssistantDelta {
  text: string;
}
```

Provider-private reasoning 不通过该事件发送。如果 provider 要求后续请求携带 reasoning state，由 Agent Core 内部处理；除非显式开启 debug logging，否则只记录安全摘要。

### `plan.updated`

```ts
interface PlanUpdated {
  steps: PlanStep[];
}
```

### `context.built`

```ts
interface ContextBuilt {
  inputTokens: number;
  maxInputTokens: number;
  cacheHitTokens?: number;
  cacheMissTokens?: number;
  includedSources: Array<{
    kind: "file" | "command" | "manifest" | "summary";
    path?: string;
    tokens: number;
    reason: string;
  }>;
}
```

### `tool.requested`

```ts
interface ToolRequested {
  toolCallId: string;
  name: string;
  risk: RiskLevel;
  argumentsPreview: unknown;
}
```

### `tool.approvalRequired`

```ts
interface ToolApprovalRequired {
  approvalId: string;
  toolCallId: string;
  risk: RiskLevel;
  title: string;
  detail: string;
  cwd?: string;
  command?: string;
  paths?: string[];
  persistable: boolean;
}
```

### `tool.started`

```ts
interface ToolStarted {
  toolCallId: string;
  name: string;
}
```

### `tool.completed`

```ts
interface ToolCompleted {
  toolCallId: string;
  name: string;
  status: "ok" | "failed";
  summary: string;
  exitCode?: number;
  stdout?: string;
  stderr?: string;
  durationMs: number;
}
```

### `patch.proposed`

```ts
interface PatchProposed {
  patchId: string;
  approvalId: string;
  files: string[];
  unifiedDiff: string;
  summary: string;
}
```

Patch 通过其中的 `approvalId` 使用 `agent.approve` 批准。

### `patch.applied`

```ts
interface PatchApplied {
  patchId: string;
  files: string[];
}
```

### `verification.started`

```ts
interface VerificationStarted {
  verificationId: string;
  command: string;
  cwd: string;
}
```

### `verification.completed`

```ts
interface VerificationCompleted {
  verificationId: string;
  status: "passed" | "failed";
  exitCode: number;
  stdout?: string;
  stderr?: string;
  durationMs: number;
}
```

## 审批状态机

```text
pending
  -> approved
      -> executing
          -> completed
          -> failed
  -> rejected
  -> canceled
  -> expired
```

规则：

- 审批请求只能使用一次。
- 被拒绝的请求不得换一个 id 重试，除非请求的操作发生实质变化。
- destructive 操作永远不可持久化。
- protocol `0.1.0` 中 network 操作不可持久化。

## 错误模型

JSON-RPC 标准错误保留标准语义。项目特定错误使用 `-32000` 到 `-32099` 范围。

| Code | Name | 含义 |
| --- | --- | --- |
| -32001 | `E_UNSUPPORTED_PROTOCOL` | client/server 协议版本不兼容。 |
| -32002 | `E_WORKSPACE_UNTRUSTED` | 当前 workspace 未信任，请求操作被禁用。 |
| -32003 | `E_RUN_NOT_FOUND` | 请求的 run 不存在于本地状态。 |
| -32004 | `E_RUN_ALREADY_ACTIVE` | 已存在冲突的 active run。 |
| -32010 | `E_INVALID_TOOL_ARGUMENTS` | tool-call 参数未通过 schema 校验。 |
| -32011 | `E_APPROVAL_NOT_FOUND` | approval id 未知、已过期或已使用。 |
| -32012 | `E_APPROVAL_DENIED` | 审批被拒绝，操作无法继续。 |
| -32020 | `E_CONTEXT_BUDGET_EXCEEDED` | 必需上下文无法放入配置的预算。 |
| -32030 | `E_PROVIDER_ERROR` | 模型 provider 返回错误或无效 stream。 |
| -32040 | `E_TOOL_EXECUTION_FAILED` | 工具在合法调用后执行失败。 |
| -32050 | `E_RUN_CANCELED` | run 已取消。 |
| -32060 | `E_INTERNAL_INVARIANT` | server 检测到内部不变量被破坏。 |

错误结构：

```json
{
  "jsonrpc": "2.0",
  "id": "req_1",
  "error": {
    "code": -32020,
    "message": "Required context exceeds token budget",
    "data": {
      "runId": "run_01",
      "requiredTokens": 1200000,
      "maxInputTokens": 1000000
    }
  }
}
```

## 端到端示例

Client 发送 turn：

```json
{
  "jsonrpc": "2.0",
  "id": "req_2",
  "method": "agent.sendTurn",
  "params": {
    "message": "Run the tests and fix failures",
    "mode": "edit"
  }
}
```

Server 接受：

```json
{
  "jsonrpc": "2.0",
  "id": "req_2",
  "result": {
    "runId": "run_01",
    "turnId": "turn_01",
    "accepted": true
  }
}
```

Server 请求命令审批：

```json
{
  "jsonrpc": "2.0",
  "method": "agent.event",
  "params": {
    "seq": 4,
    "time": "2026-05-20T14:00:05Z",
    "type": "tool.approvalRequired",
    "runId": "run_01",
    "turnId": "turn_01",
    "payload": {
      "approvalId": "approval_01",
      "toolCallId": "tool_01",
      "risk": "exec",
      "title": "Run tests",
      "detail": "Execute cargo test --workspace",
      "cwd": "C:/workspace/deepseek-coder",
      "command": "cargo test --workspace",
      "persistable": false
    }
  }
}
```

Client 批准：

```json
{
  "jsonrpc": "2.0",
  "id": "req_3",
  "method": "agent.approve",
  "params": {
    "approvalId": "approval_01"
  }
}
```

随后 server 继续发送 `tool.started`、`tool.completed`、可选的 `patch.proposed`、verification events，最后发送 `run.completed`。

## 运行日志（Run Log）要求

Server 必须能够通过以下信息重建 run：

- initialize metadata
- user turns
- event stream
- approvals and rejections
- tool results
- patch proposals and applied patches
- verification results

Run log 持久化前必须脱敏密钥。

## 实现说明

- `packages/protocol` 定义与本文档匹配的 TypeScript 类型。
- `crates/agent-rpc` 负责 Rust 协议结构和 JSON-RPC framing。
- 后续应增加兼容性测试，验证 Rust 和 TypeScript 的协议定义一致。
