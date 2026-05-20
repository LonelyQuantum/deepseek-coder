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
