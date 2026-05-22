# 工具系统

状态：`0.1.0` 设计已确定，Phase 1 基础执行层已实现。

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

文件内容摘要字段如 `sha256` 属于 manifest 和缓存增强项，当前 Phase 1 `read_file` 执行结果尚不返回该字段。

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
- Rust 工具执行层：`crates/agent-core/src/tool_execution.rs`。
- Rust 审批类型：`crates/agent-core/src/approval.rs`。
- TypeScript 协议类型：`packages/protocol/src/index.ts`。

后续 `crates/agent-rpc` 应把 Rust 类型序列化为 `docs/json-rpc-protocol.md` 中定义的 JSON-RPC 事件。

## Phase 1 实现范围

`WorkspaceToolExecutor` 当前提供：

- `read_file`：只读取 workspace 内 UTF-8 文本文件，支持 1-based 行范围。
- `search`：通过 `rg --json --fixed-strings` 搜索，默认排除 `.git/`、`.secrets/`、`.secret/`、`.env*`、`node_modules/` 和 `target/`。
- `apply_patch`：应用受限 unified diff，要求 patch 实际文件集合与 `expectedFiles` 完全一致，并返回 reverse patch。
- `shell`：在 workspace 内执行非交互式命令，支持超时，返回 exit code、stdout、stderr 和耗时。
- `git_status`：读取 `git status --short --branch` 或普通 `git status`。
- `git_diff`：读取 unstaged 或 staged diff，支持限定 workspace-relative 路径。

路径规则：

- 工具参数使用 workspace-relative path。
- 绝对路径、`..` 路径和解析到 workspace 外的路径都会失败。
- `.git/`、`.secrets/`、`.secret/`、`.agents/`、`.codex/`、`.env` 和 `.env.*` 被视为敏感路径，读写工具默认拒绝访问。

当前实现暂不包含 workspace manifest、LSP diagnostics 和 plan update 的执行逻辑；它们仍只有 schema 和静态风险定义。

当前执行层还没有接入 Agent Turn Loop、审批引擎、run log 或 JSON-RPC 事件流。写入、命令执行和网络风险升级仍需要在编排层实现后才能对用户开放自动化流程。

## 后续增强

### 工具注册与协议一致性

- 为 Rust 和 TypeScript 的每个工具补齐具体 `resultSchema`，替换当前通用 `statusResultSchema`。
- 增加 Rust/TypeScript schema 兼容性测试，避免协议文档、Rust 类型和 `packages/protocol` 分叉。
- 在 `crates/agent-rpc` 中把工具请求、审批请求、工具结果和 patch 事件序列化为 `docs/json-rpc-protocol.md` 定义的事件。

### 路径与敏感信息

- 将当前静态敏感路径拒绝规则扩展为可配置规则，合并 `.gitignore`、用户 ignore 配置、常见密钥文件名和平台密钥目录。
- 在工具结果进入 run log 或 prompt 前增加统一脱敏层，覆盖 stdout、stderr、diff、搜索结果和读取文件内容。
- 对大文件、二进制文件和非 UTF-8 文件给出结构化错误或专门的 metadata 结果，而不是把它们交给文本工具处理。

### `read_file`

- 增加 `sha256`、字节长度、编码信息和内容截断元数据。
- 支持按 token 预算或语法边界读取片段，避免长文件被随意切断。
- 增加文件快照 id，方便 run log 复现“读取时看到的内容”。

### `search`

- 增加 regex 模式、glob include/exclude、上下文行、文件类型过滤和排序策略。
- 支持流式解析 `rg --json`，达到 `maxResults` 后提前停止进程，减少大仓库扫描开销。
- 改进 `truncated` 判定，使其只基于真实 match 数量，而不是命令输出行数。

### `apply_patch`

- 当前实现只支持受限 unified diff；后续需要支持更完整的 git patch 语法，包括 rename、copy、mode change 和更严格的 no-newline 语义。
- 增加 patch 预览、hunk 级审批、冲突诊断和失败时的精确 hunk mismatch 信息。
- 用修改前快照生成 reverse patch，并在 run log 中保存 patch id、审批 id 和可审计回滚信息。
- 明确二进制文件和生成文件策略，避免文本 patch 意外改写不可审计内容。

### `shell`

- 在执行前加入命令风险分类：网络、破坏性、依赖安装、发布、远程 git 操作等必须升级审批。
- 限制输出大小并记录截断原因，避免 stdout/stderr 把密钥或超大日志直接带入 run log。
- 记录环境变量差异，但默认隐藏或脱敏敏感变量。
- 后续按平台分别实现更强的 sandbox 策略；Windows、Linux 和 macOS 不能假设具备相同隔离能力。

### `git_status` / `git_diff`

- 增加 staged/unstaged/untracked 的结构化摘要，便于 UI 展示和上下文构建。
- 解析 rename、delete、binary diff、submodule 和 worktree 状态。
- 增加 pathspec 校验与 diff 大小限制，避免把超大 diff 直接塞入模型上下文。

### 尚未实现的内置工具

- `workspace_manifest`：应生成长上下文的稳定骨架，包含 ignore 规则、语言、大小、hash、token 估算和风险标记。
- `lsp_diagnostics`：应能从 VS Code 或独立语言服务器读取 Problems/diagnostics，并保留来源、范围和严重级别。
- `plan_update`：应由 Agent Core 写入 run log，并通过 JSON-RPC 事件同步给 CLI/TUI/VS Code。
