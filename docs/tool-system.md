# 工具系统

状态：`0.1.0` 设计已确定，基础类型实现中。

工具系统通过显式 schema 和类型化结果向 Agent Core 暴露工作区操作。模型不得直接执行文件写入、shell 命令或网络访问；它只能请求工具，工具请求必须经过 schema 校验和审批策略。

## 设计目标

- 工具名称稳定。
- 工具参数使用 JSON Schema 描述。
- 工具结果结构化，可进入 run log。
- 每个工具都有风险等级和审批要求。
- 工具失败必须显式返回，不通过后处理掩盖。
- 工具结果写入日志或 prompt 前必须脱敏密钥。

## 风险等级

风险等级与 `docs/approval-model.md` 保持一致：

- `read`
- `write`
- `exec`
- `network`
- `destructive`

工具定义中的风险等级是最低风险。Agent Core 可以基于具体参数把风险升级，但不得降级。

## 工具定义结构

TypeScript 表示：

```ts
interface ToolDefinition {
  name: ToolName;
  description: string;
  risk: RiskLevel;
  approval: ApprovalRequirement;
  argumentSchema: JsonSchema;
  resultSchema: JsonSchema;
}
```

Rust 表示：

```rust
pub struct ToolDefinition {
    pub name: ToolName,
    pub description: &'static str,
    pub risk: RiskLevel,
    pub approval: ApprovalRequirement,
    pub argument_schema: &'static str,
    pub result_schema: &'static str,
}
```

## 通用结果字段

所有工具结果至少包含：

```json
{
  "status": "ok",
  "summary": "human readable summary"
}
```

失败时：

```json
{
  "status": "failed",
  "summary": "human readable failure",
  "errorCode": "E_TOOL_EXECUTION_FAILED"
}
```

命令类工具可以额外包含：

- `stdout`
- `stderr`
- `exitCode`
- `durationMs`

文件类工具可以额外包含：

- `path`
- `content`
- `lineCount`
- `sha256`

## 内置工具

### `workspace_manifest`

生成工作区 manifest。

风险：`read`。

审批：`none`。

参数：

- `root`：workspace 根目录，省略时使用初始化时的 `workspaceRoot`。
- `respectGitignore`：是否遵守 `.gitignore`。

结果：

- `entries`：文件条目列表。
- `ignoredCount`：被忽略文件数量。

### `read_file`

读取 UTF-8 文本文件并保留行信息。

风险：`read`。

审批：`none`。

参数：

- `path`：workspace-relative path。
- `startLine`：可选，1-based。
- `endLine`：可选，1-based。

结果：

- `path`
- `content`
- `lineCount`
- `sha256`

### `search`

使用 ripgrep 搜索。

风险：`read`。

审批：`none`。

参数：

- `query`：搜索字符串。
- `paths`：可选路径列表。
- `caseSensitive`：是否大小写敏感。
- `maxResults`：最大结果数。

结果：

- `matches`：匹配列表。
- `truncated`：是否因为 `maxResults` 被截断。

### `apply_patch`

应用统一 diff patch。该工具是文本写入的唯一入口。

风险：`write`。

审批：`required`。

参数：

- `unifiedDiff`：统一 diff。
- `expectedFiles`：预期修改文件列表。

结果：

- `files`：实际修改文件列表。
- `reversePatch`：用于回滚的反向 patch。

### `shell`

执行非交互式命令。

风险：`exec`。

审批：`required`。

参数：

- `command`：命令字符串。
- `cwd`：workspace-relative 工作目录，省略时使用 workspace root。
- `timeoutMs`：超时时间。

结果：

- `exitCode`
- `stdout`
- `stderr`
- `durationMs`

说明：`shell` 的静态风险是 `exec`。涉及网络或破坏性操作的命令必须由 Agent Core 在执行前升级审批风险；协议 `0.1.0` 不允许自动降级或静默执行。

### `git_status`

读取 git 状态。

风险：`read`。

审批：`none`。

参数：

- `porcelain`：是否输出 porcelain 格式。

结果：

- `branch`
- `entries`

### `git_diff`

读取 git diff。

风险：`read`。

审批：`none`。

参数：

- `staged`：读取 staged diff。
- `paths`：可选路径列表。

结果：

- `unifiedDiff`
- `files`

### `lsp_diagnostics`

读取语言服务器或编辑器 diagnostics。

风险：`read`。

审批：`none`。

参数：

- `paths`：可选路径列表。

结果：

- `diagnostics`

### `plan_update`

更新 Agent 当前计划。

风险：`read`。

审批：`none`。

参数：

- `steps`：计划步骤列表。

结果：

- `accepted`

## 实现位置

- Rust 基础类型：`crates/agent-core/src/tool.rs`。
- Rust 审批类型：`crates/agent-core/src/approval.rs`。
- TypeScript 协议类型：`packages/protocol/src/index.ts`。

后续 `crates/agent-rpc` 应把 Rust 类型序列化为 `docs/json-rpc-protocol.md` 中定义的 JSON-RPC 事件。
