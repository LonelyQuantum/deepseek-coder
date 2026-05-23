# 上下文胶囊（Context Capsule）

状态：草案，Phase 1 基础 Context Builder 已实现。

Context Capsule 是一次模型回合的结构化输入包。它面向 DeepSeek 的长上下文和上下文缓存能力设计。

## 目标

- 稳定内容靠近 prompt 前缀。
- 易变内容靠近 prompt 后缀。
- 每个片段都能追溯来源。
- 发送前计算 token 预算。
- 必需上下文无法放入预算时显式停止。

## 结构

```text
system policy
project rules
user task
workspace manifest
git status and diff summary
selected file contents
tool results
active plan
acceptance criteria
previous run summary
```

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

## Phase 1 当前实现

实现位置：`crates/agent-core/src/context.rs`。

当前 Context Builder 提供：

- `ContextItem`：表示用户任务、项目规则、git 状态、文件、工具结果、计划等上下文片段。
- 固定优先级排序：system policy、project rules、user task、manifest、git、file、tool result、plan 等来源按稳定顺序进入模型输入。
- 工作区相对路径校验：拒绝绝对路径、Windows 盘符路径和 `..` 父目录路径。
- 来源冲突检查：`SystemPolicy`、`UserTask`、`WorkspaceManifest`、git 状态、计划等 singleton 类型只能出现一次；同一个文件路径或 command id 不能重复进入同一份上下文。
- token 预算控制：必需上下文超预算时显式失败；可选上下文超预算时跳过并写入 omitted source 报告。
- token 报告：输出 `inputTokens`、`maxInputTokens`、included sources、omitted sources 和估算器说明。
- 统一脱敏：进入上下文前复用 Run Log 的基础脱敏规则，避免把明显的 secret-like 文本写入 prompt。

当前 token 统计使用 `utf8_bytes` 估算器：它用 UTF-8 字节数作为确定性估算，不是 DeepSeek tokenizer 的精确 token 数。报告中的 `estimator.exact` 必须为 `false`，前端和后续 turn loop 不能把它展示成精确模型 token。

当前 Context Builder 已接入基础 Agent Turn Loop。Turn Loop 会收集用户任务和调用方提供的上下文条目，生成 provider 请求输入，并把 `context.built` 事件写入 Run Log。基础 RPC 事件桥接已能把该事件发送为 JSON-RPC notification；后续仍需要接入完整 request loop，并扩展 workspace manifest、git 状态、选中文件、工具结果和计划步骤的自动收集。

## 与工具系统的衔接

Context Capsule 不直接扫描工作区，而是消费工具系统和 run log 产生的结构化结果：

- `workspace_manifest` 提供稳定文件骨架和风险标记。
- `git_status` / `git_diff` 提供当前工作区变化摘要。
- `read_file` 和 `search` 提供已审计的文件片段与来源路径。
- `lsp_diagnostics` 提供编辑器或语言服务器诊断。
- 工具结果进入上下文前必须经过脱敏、大小限制和来源标注。

## 后续增强

- 实现 workspace manifest，记录 path、kind、bytes、hash、token 估算、git object id、ignore 原因和风险标记。
- 接入真实 provider tokenizer，为 DeepSeek 1M 上下文和较小 provider 同时生成更接近模型实际计费的预算报告。
- 根据 Agent Turn Loop 的真实输入形态，评估是否允许多个 project rules、diagnostics 或分片后的 git diff/file chunk，并把允许重复的来源类型写入 schema。
- 在接入真实模型回合后验证当前 Markdown 元数据头格式的 prompt 效果；如果需要，抽象为可版本化的 prompt renderer。
- 建立稳定前缀策略，把项目规则、manifest 和固定文件快照放在可缓存区域，把最新用户消息、工具输出和计划放在后缀。
- 增加超预算诊断，明确列出哪些必需内容无法纳入、原因是什么、用户可以如何缩小任务。
- 给大文件和生成文件增加专门策略，避免把不可审计内容直接塞入 prompt。
