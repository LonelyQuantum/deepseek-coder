# 审批模型

状态：`0.1.0` 设计已确定，基础类型已实现，审批编排尚未实现。

审批模型用于保护工作区，避免未审阅的写入、命令执行、网络访问和破坏性操作。审批是 Agent Core 的核心安全边界，不由前端单独实现。

## 风险等级

| 等级 | 英文标识 | 示例 | 默认策略 |
| --- | --- | --- | --- |
| 读取 | `read` | read file、search、git status | 自动允许 |
| 写入 | `write` | apply patch、格式化已跟踪文件 | 需要审批 |
| 执行 | `exec` | 测试、构建、lint 命令 | 需要审批 |
| 网络 | `network` | 下载依赖、远程 API | 需要审批 |
| 破坏性 | `destructive` | 删除、reset、清理未跟踪文件 | 总是审批 |

默认策略：

```text
read        -> none
write       -> required
exec        -> required
network     -> required
destructive -> always_required
```

## 审批要求

审批要求使用三个稳定值：

- `none`：无需审批。
- `required`：每次操作前审批，可以在未来支持安全的 session/workspace 持久批准。
- `always_required`：每次都必须审批，不允许持久化。

`destructive` 风险必须使用 `always_required`。

## 审批请求

审批请求必须展示：

- `approvalId`
- `toolCallId`
- 工具名
- 风险等级
- 标题
- 详细说明
- 工作目录
- 精确命令或文件路径
- 是否允许持久化

前端只能显示和提交用户决定。Agent Core 负责判断请求是否有效、是否过期、是否可持久化。

## 状态机

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

允许的转换：

| From | To |
| --- | --- |
| `pending` | `approved` |
| `pending` | `rejected` |
| `pending` | `canceled` |
| `pending` | `expired` |
| `approved` | `executing` |
| `executing` | `completed` |
| `executing` | `failed` |

其他转换都是协议错误或内部状态错误。

## 拒绝审批

审批被拒绝时：

- Agent Core 记录拒绝结果。
- 原请求失效。
- Agent Core 不得用相似命令或相同 patch 绕过拒绝。
- Agent Core 只能请求用户选择其他路径、继续只读工作或停止 run。

## 持久化规则

协议 `0.1.0` 中：

- `read` 不需要持久审批。
- `write` 可以在未来支持 session/workspace 持久化，但默认不启用。
- `exec` 默认不持久化。
- `network` 不允许持久化。
- `destructive` 永远不允许持久化。

## 实现位置

- Rust：`crates/agent-core/src/approval.rs`。
- TypeScript：`packages/protocol/src/index.ts`。
- JSON-RPC 事件：`docs/json-rpc-protocol.md` 中的 `tool.approvalRequired`、`agent.approve`、`agent.reject`。

当前 Rust 和 TypeScript 已定义风险等级、审批要求、持久化枚举和状态机转换规则。尚未实现的是运行时审批队列、过期策略、持久批准存储、前端交互和工具执行前的动态风险升级。

## 后续增强

- 实现 Agent Core 审批引擎：创建审批请求、等待用户决定、处理过期、记录拒绝和阻止绕过。
- 实现命令与 patch 的动态风险升级；例如 `shell` 的静态风险是 `exec`，但下载依赖、访问网络、删除文件或远程 git 操作必须升级到更高风险。
- 增加 session/workspace 持久批准，但只允许明确可持久化的低风险操作使用；`network` 和 `destructive` 不允许持久化。
- 把审批请求和审批结果写入 run log，并通过 JSON-RPC 事件同步给 CLI/TUI/VS Code。
- 增加跨前端一致性测试，确保同一工具请求在 CLI、TUI 和 VS Code 中展示的风险、路径、命令和持久化选项一致。
