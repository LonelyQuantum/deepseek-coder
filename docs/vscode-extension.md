# 编辑器插件（VS Code Extension）

状态：草案。

VS Code 插件是 `deepseek-coder` 的一等前端。它应通过 JSON-RPC server 复用 Rust Agent Core。

## 职责

- 启动并监管 Agent RPC Server。
- 渲染 chat 和 run events。
- 展示计划、tool call 和审批请求。
- 使用 VS Code 原生 diff editor 展示 patch。
- 从 Problems 面板读取 diagnostics。
- 尊重 Workspace Trust。
- 暴露 provider、model 和 approval policy 设置。

## 非职责

插件不应实现自己的 Agent loop、context builder 或 tool execution engine。

## 当前骨架

当前插件只注册 `deepseek-coder.openChat`，并提示 workspace 已就绪。RPC 集成尚未实现。
