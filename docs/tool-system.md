# 工具系统

状态：草案。

工具系统通过显式 schema 和类型化结果向 Agent Core 暴露工作区操作。

## 初始工具

- `workspace_manifest`：生成 workspace manifest。
- `read_file`：读取文本文件并保留行信息。
- `search`：使用 ripgrep 搜索。
- `apply_patch`：应用文本 patch 并记录 reverse patch。
- `shell`：在需要时经审批后执行命令。
- `git_status`：读取 git status。
- `git_diff`：读取 git diff。
- `lsp_diagnostics`：收集编辑器或语言服务器诊断。
- `plan_update`：更新当前计划。

## 工具契约

每个工具都需要：

- 稳定名称
- 参数 JSON Schema
- 风险等级
- 执行结果 schema
- 日志策略
- 审批要求

## 结果结构

工具结果应包含：

- 状态
- 摘要
- 结构化数据
- 需要时包含 stdout/stderr
- 耗时
- 需要时包含 exit code

结果写入日志或 prompt 前必须脱敏密钥。
