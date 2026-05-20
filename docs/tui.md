# 终端界面（TUI）

状态：草案。

TUI 是 `deepseek-coder` 的终端前端。它应支持键盘驱动的代码工作流，同时与 VS Code 插件共享 Agent Core 行为。

## 计划视图

- Chat：对话和流式模型输出。
- Plan：当前计划和步骤状态。
- Diff：文件级和 hunk 级拟议修改。
- Tools：tool call、审批、命令输出和 exit code。
- Context：token 预算、纳入来源和 cache 使用情况。
- Settings：provider、model 和 approval policy。

## 原则

- 工具执行放在 Rust core 中，不放在 TUI view code 中。
- 审批必须显式且可审计。
- 保留 run id 和 resume point。
- 使用确定性的键盘快捷键。
