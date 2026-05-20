# 架构决策记录 0001：使用 Rust 和 TypeScript

状态：已接受。

## 背景

`deepseek-coder` 需要可靠的本地执行核心，也需要 VS Code 插件前端。

## 决策

使用 Rust 编写 Agent Core、RPC server、CLI、TUI、本地工具和 run log。使用 TypeScript 编写 VS Code 插件和共享前端协议类型。

## 影响

- Rust 适合本地文件操作、命令执行、结构化错误和跨平台二进制。
- TypeScript 符合 VS Code 插件生态。
- 项目需要同时管理 Cargo workspace 和 pnpm workspace。
- 跨语言边界必须显式，因此 JSON-RPC 成为核心接口。
