# 上下文胶囊（Context Capsule）

状态：草案。

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

Phase 1 只实现基础 Context Builder，不直接追求完整 1M Context Capsule：

- 输入来源限定为用户任务、项目规则、git 状态、显式选中文件、必要工具结果和当前计划。
- 每个上下文片段必须记录来源、纳入原因和 token 统计来源。
- 如果 token 统计只是估算，必须在结果中显式标注估算器；不能把估算当作精确值。
- 超预算时显式失败，并列出缺失内容，而不是静默丢弃必要上下文。

完整 workspace manifest、稳定前缀、缓存命中统计和大仓库基准属于 Phase 2。

## 与工具系统的衔接

Context Capsule 不直接扫描工作区，而是消费工具系统和 run log 产生的结构化结果：

- `workspace_manifest` 提供稳定文件骨架和风险标记。
- `git_status` / `git_diff` 提供当前工作区变化摘要。
- `read_file` 和 `search` 提供已审计的文件片段与来源路径。
- `lsp_diagnostics` 提供编辑器或语言服务器诊断。
- 工具结果进入上下文前必须经过脱敏、大小限制和来源标注。

## 后续增强

- 实现 workspace manifest，记录 path、kind、bytes、hash、token 估算、git object id、ignore 原因和风险标记。
- 接入 token 计数器，为 DeepSeek 1M 上下文和较小 provider 同时生成预算报告。
- 建立稳定前缀策略，把项目规则、manifest 和固定文件快照放在可缓存区域，把最新用户消息、工具输出和计划放在后缀。
- 增加超预算诊断，明确列出哪些必需内容无法纳入、原因是什么、用户可以如何缩小任务。
- 给大文件和生成文件增加专门策略，避免把不可审计内容直接塞入 prompt。
