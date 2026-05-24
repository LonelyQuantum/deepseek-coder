# 上下文胶囊（Context Capsule）

状态：草案，Phase 1 基础 Context Builder 已实现；Phase 2 开发计划已收敛，下一步进入 Context Capsule 数据模型与 Manifest v0。

Context Capsule 是一次模型回合的结构化输入包。它面向 DeepSeek 的长上下文和上下文缓存能力设计。

## 目标

- 稳定内容靠近 prompt 前缀。
- 易变内容靠近 prompt 后缀。
- 每个片段都能追溯来源。
- 发送前计算 token 预算。
- 必需上下文无法放入预算时显式停止。

## 结构

```text
[StablePrefix]
system policy
project rules
workspace manifest summary
stable file snapshots

[DynamicPrelude]
git status and diff summary
current diagnostics summary

[TurnSuffix]
user task
selected file contents
explicit attachments
tool results
active plan
acceptance criteria
previous run summary
```

Phase 2 不再只把 Context Capsule 视为一段拼好的 prompt 字符串。Agent Core 应先构建结构化结果，再由稳定 renderer 生成 provider 输入：

```rust
struct ContextCapsule {
    sections: Vec<ContextSection>,
    rendered: String,
    token_report: ContextTokenReport,
}

struct ContextSection {
    placement: CachePlacement,
    items: Vec<ContextItem>,
}

enum CachePlacement {
    StablePrefix,
    DynamicPrelude,
    TurnSuffix,
}
```

`CachePlacement` 描述 prompt 布局位置，不等同于现有 `ContextItemKind::priority()`。`priority()` 继续用于同一区域内的确定性排序；`CachePlacement` 用于决定内容是否进入稳定前缀、动态前导区或当前 turn 后缀。

默认 placement 建议：

| 来源 | 默认 placement |
| --- | --- |
| system policy、project rules、workspace manifest summary、稳定文件快照 | `StablePrefix` |
| git status summary、git diff summary、diagnostics summary | `DynamicPrelude` |
| user task、selected files、explicit attachments、tool results、active plan、previous run summary | `TurnSuffix` |

## Workspace Manifest v0

Manifest 是长上下文的骨架。Phase 2 使用结构化 JSON 作为内部表示，并用 canonical serialization 计算 `manifestHash`；prompt 中只放紧凑、稳定、可读的 manifest summary。

第一版字段：

```json
{
  "manifestVersion": 1,
  "manifestHash": "sha256:...",
  "workspaceRoot": "<redacted-or-local-only>",
  "maxEntries": 500,
  "totalDiscoveredFiles": 1234,
  "includedFiles": 500,
  "entries": [
    {
      "path": "src/lib.rs",
      "kind": "rust",
      "sizeBytes": 1200,
      "sha256": "...",
      "git": "working-tree",
      "risk": "source"
    }
  ],
  "omitted": [
    {
      "reason": "max_entries_exceeded",
      "count": 734
    }
  ]
}
```

`maxEntries` 默认 500。裁剪时优先保留显式选中、已变化、根层级和源码/配置文件；省略项必须进入 token/来源报告，不能静默丢弃。

忽略规则分三层：

1. **硬安全排除**：`.git/`、`.secrets/`、密钥、证书、浏览器配置等，不能被用户规则重新纳入。
2. **默认工程排除**：`target/`、`node_modules/`、`dist/`、`build/` 等，可在后续配置中调整。
3. **用户上下文排除**：`.gitignore` 与 `.deepseek-coderignore`。`.deepseek-coderignore` 复用 gitignore 语法和 `!` negation，不引入额外自定义 include 语法。

## 来源元数据

每个上下文条目应携带：

- 来源路径或 command id
- 内容类型
- 字节长度
- token 估算
- 时间戳或 git object id
- 纳入原因

## 预算规则

必需上下文不能被静默丢弃。如果任务需要的材料无法放入预算，Agent Core 应报告缺失内容，并要求用户缩小任务范围或允许分阶段执行。

## 分阶段落地

Phase 1 已实现基础 Context Builder，不直接追求完整 1M Context Capsule：

- 输入来源限定为用户任务、项目规则、git 状态、显式选中文件、必要工具结果和当前计划。
- 每个上下文片段必须记录来源、纳入原因和 token 统计来源。
- 如果 token 统计只是估算，必须在结果中显式标注估算器；不能把估算当作精确值。
- 超预算时显式失败，并列出缺失内容，而不是静默丢弃必要上下文。

完整 workspace manifest、稳定前缀、缓存命中统计和大仓库基准属于 Phase 2。

Phase 2 按 4 个增量轮次落地：

| 阶段 | 内容 | 默认验收 |
| --- | --- | --- |
| Phase 2a | `read_file` 增加 `sha256` / `sizeBytes`；定义 `ContextCapsule`、`ContextSection`、`CachePlacement` 和稳定 renderer；实现 workspace manifest v0；接入 manifest summary 与 `context.built` payload。 | 离线 fixture 仓库 manifest、hash、ignore、truncation 和渲染稳定性测试。 |
| Phase 2b | 建立 `TokenEstimator` trait，保留 `utf8_bytes` 默认估算器，新增 `CalibratedEstimator`，实现三层 cache-friendly prompt layout。 | 修改 `TurnSuffix` 不改变 `StablePrefix`；token estimator metadata 明确标注 `exact=false`。 |
| Phase 2c | 接入 `agent.sendTurn.attachments` 的 file/selection/diagnostic；新增 `provider.completed` usage/cache 事件；建立 DeepSeek cache hit/miss ignored live 实验。 | attachment 越界、重复、超大小限制测试；live cache 实验手动运行。 |
| Phase 2d | 200K/500K/900K 样例仓库验收；超预算解释；Run Log 体积/截断/脱敏边界；tool call JSON Schema 通用校验层。 | 大仓库 ignored/manual benchmark；schema validation 在 typed deserialization 前执行。 |

## Phase 1 当前实现

实现位置：`crates/agent-core/src/context.rs`。

当前 Context Builder 提供：

- `ContextItem`：表示用户任务、项目规则、git 状态、文件、工具结果、计划等上下文片段。
- 固定优先级排序：system policy、project rules、user task、manifest、git、file、tool result、plan 等来源按稳定顺序进入模型输入。Phase 2 会在该排序之外增加 `CachePlacement`，避免把展示顺序与缓存前缀混用。
- 工作区相对路径校验：拒绝绝对路径、Windows 盘符路径和 `..` 父目录路径。
- 来源冲突检查：`SystemPolicy`、`UserTask`、`WorkspaceManifest`、git 状态、计划等 singleton 类型只能出现一次；同一个文件路径或 command id 不能重复进入同一份上下文。
- token 预算控制：必需上下文超预算时显式失败；可选上下文超预算时跳过并写入 omitted source 报告。
- token 报告：输出 `inputTokens`、`maxInputTokens`、included sources、omitted sources 和估算器说明。
- 统一脱敏：进入上下文前复用 Run Log 的基础脱敏规则，避免把明显的 secret-like 文本写入 prompt。

当前 token 统计使用 `utf8_bytes` 估算器：它用 UTF-8 字节数作为确定性估算，不是 DeepSeek tokenizer 的精确 token 数。报告中的 `estimator.exact` 必须为 `false`，前端和后续 turn loop 不能把它展示成精确模型 token。

当前 Context Builder 已接入基础 Agent Turn Loop。Turn Loop 会收集用户任务和调用方提供的上下文条目，生成 provider 请求输入，并把 `context.built` 事件写入 Run Log。基础 RPC 事件桥接和 `AgentTurnLoopRpcHandler` 已能把该事件发送为 JSON-RPC notification；后续仍需要扩展 workspace manifest、git 状态、选中文件、工具结果和计划步骤的自动收集。

## 与工具系统的衔接

Context Capsule 不直接扫描工作区，而是消费工具系统和 run log 产生的结构化结果：

- `workspace_manifest` 提供稳定文件骨架、摘要、忽略原因、manifest hash 和风险标记。
- `git_status` / `git_diff` 提供当前工作区变化摘要。
- `read_file` 和 `search` 提供已审计的文件片段与来源路径；Phase 2a 会让 `read_file` 返回 `sha256` 和 `sizeBytes`，供 manifest 和工具结果一致性校验使用。
- `lsp_diagnostics` 提供编辑器或语言服务器诊断。
- 工具结果进入上下文前必须经过脱敏、大小限制和来源标注。

## 后续增强

- 实现 workspace manifest v0，记录 path、kind、sizeBytes、sha256、manifestHash、git object id、ignore 原因和风险标记。
- 建立 `TokenEstimator` trait 和 `CalibratedEstimator`，为 DeepSeek 1M 上下文和较小 provider 同时生成更接近模型实际计费的预算报告；在没有官方 tokenizer 或明确等价实现前，估算器必须标注 `exact=false`。
- 根据 Agent Turn Loop 的真实输入形态，评估是否允许多个 project rules、diagnostics 或分片后的 git diff/file chunk，并把允许重复的来源类型写入 schema。
- 在接入真实模型回合后验证当前 Markdown 元数据头格式的 prompt 效果；如果需要，抽象为可版本化的 prompt renderer。
- 按 `CachePlacement::{StablePrefix, DynamicPrelude, TurnSuffix}` 建立缓存友好 prompt 布局，并用 DeepSeek cache hit/miss 字段做 ignored live 验证。
- 接入 `agent.sendTurn.attachments`，支持 file、selection/explicit content 和 diagnostic 等显式上下文来源。
- 新增 `provider.completed` 事件，记录模型、duration、usage、cache hit/miss 和 stream 摘要。
- 增加超预算诊断，明确列出哪些必需内容无法纳入、原因是什么、用户可以如何缩小任务。
- 给大文件和生成文件增加专门策略，避免把不可审计内容直接塞入 prompt。
