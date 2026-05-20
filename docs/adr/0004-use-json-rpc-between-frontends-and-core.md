# 架构决策记录 0004：前端和 Core 之间使用 JSON-RPC

状态：已接受。

## 背景

CLI、TUI 和 VS Code 应共享同一个 Agent Core，而不是分别实现 agent 行为。

## 决策

前端与 Rust Agent RPC Server 之间使用 JSON-RPC，并通过结构化事件传输流式输出、计划、工具、审批、patch 和 run 完成状态。

## 影响

- 前端保持轻量，主要负责渲染状态。
- 各 UI 表面的 Agent 行为保持一致。
- 需要维护协议版本和 schema 兼容性。
- run log 可以基于同一事件流构建。
