# 路线图

状态：草案，随 Phase 1 实现持续更新。

本文档把 README 中的大阶段拆成更可执行的优先级。README 保留项目入口和高层计划；这里记录跨模块的落地顺序、取舍和验收重点。

## 定位

`deepseek-coder` 不以第一阶段覆盖通用 AI 编程工具的全部功能为目标。项目的核心差异是：

- DeepSeek V4 1M 上下文优先。
- `reasoning_content`、上下文缓存和长输出能力一等支持。
- Rust Agent Core 供 CLI/TUI/VS Code 共用。
- 可复现 run log、显式审批和自由软件治理。
- 中文工作流友好。

因此短期不和成熟 VS Code 扩展正面拼功能覆盖率，而是先做出一个可审计、可复现、能稳定闭环的小型 Agent。

## P0：Phase 1 MVP 收敛

目标：完成一个最小但真实可用的 Agent 回合。

已完成：

- DeepSeek API adapter。
- 流式响应解析。
- `reasoning_content` 状态机。
- read/search/apply_patch/shell/git 基础工具执行层。
- 基础 Run Log 存储层。
- 基础 Context Builder 与 token 预算报告。
- 基础工具注册表 Rust/TypeScript 兼容性 fixture。
- Agent Turn Loop 基础编排和 fake provider 集成测试骨架。
- Agent RPC Server 最小 stdio 事件桥接。
- CLI `run` 最小闭环，支持 DeepSeek provider、fixture provider、run log 摘要、JSON event 重放和显式 verification command。
- 本地 fixture 端到端 smoke test，覆盖 CLI、Turn Loop、工具执行、Run Log 和 JSON-RPC event 输出。
- CLI 审查修复：DeepSeek provider 改为专用 current-thread runtime，fixture provider 改为响应队列，verification 输出在写入 run log 前脱敏。
- 进程级 CLI fixture smoke test：从编译出的 `deepseek-coder` 二进制启动，验证 CLI、Turn Loop、Run Log 和 JSON-RPC event 输出的最小闭环。
- TurnProvider async / streaming 边界：`TurnProvider::complete_stream` 返回异步事件流，支持 `assistant.delta` 与最终 `Completed` 响应。
- CLI DeepSeek provider streaming wrapper：CLI provider 通过 `create_chat_completion_stream` 聚合 content、`reasoning_content` 和 tool calls，并把 content delta 写入 run log。
- 真实 provider streaming 联网验收：`deepseek_cli_live` 从编译出的 CLI 二进制启动真实 DeepSeek provider，验证 `stream: true` 的 `assistant.delta` 和最终 `run.completed`。
- streaming tool call 增量拼装验证：adapter 已区分 `ChatToolCallDelta` 与完整 `ChatToolCall`，`ChatToolCallAccumulator` 会按 `index` 拼接 arguments 并拒绝缺失或冲突元数据；`live_streaming_tool_call_accumulator_smoke_test` 已用真实 DeepSeek streaming 验收工具调用 delta 形态。
- Agent RPC Server 双向 request loop：`agent-rpc` 已支持 newline-delimited JSON-RPC request 读取、初始化顺序检查、`agent.initialize` / `agent.sendTurn` / `agent.resume` 分发、response/error 写回，以及 handler 返回事件的 `agent.event` 有序输出。
- RPC/CLI 实时事件输出：`AgentTurnLoop::run_turn_with_event_sink` 会在 Run Log 事件追加成功后立即调用 `TurnEventSink`；`StdioEventBridge` 已实现该接口，CLI `--json` 输出顺序与本地 `events.jsonl` 的 `seq` 一致，不再等 run 完成后批量回放。
- CLI/RPC/TUI/VS Code 审批基础：Turn Loop 会写入 `tool.approvalRequired` 和 `tool.approvalResolved`；CLI 二进制支持 stdin/stderr 交互式 y/n 审批；RPC request loop 已分发 `agent.approve` / `agent.reject`；TypeScript 协议类型已补齐；TUI prompt 状态机和 VS Code modal approval adapter 已有测试覆盖。
- 真实 RPC Turn Loop handler：`AgentTurnLoopRpcHandler` 已通过 provider factory 复用 Core Turn Loop，`agent.sendTurn` 会创建 run log、驱动 provider 和工具执行，并把结果事件交给 request loop；CLI `rpc` 子命令已提供 stdio 入口。
- RPC 真实审批等待队列：`AgentTurnLoopRpcHandler` 会在 `tool.approvalRequired` 处登记 pending approval，后台 Turn Loop worker 等待 `agent.approve` / `agent.reject`，批准后继续执行工具，拒绝后记录 `tool.approvalResolved` 和 `run.failed`。
- RPC 审批超时/取消：pending approval 已记录过期时间；`agent.cancel` 会取消等待审批的 active run，超时会自动解析为 expired，两者都会记录 `tool.approvalResolved` 和 `run.canceled`。
下一步：

- Run Log 写入串行化：Turn Loop / RPC 层必须保证同一 run 的事件由单 writer 或同步队列按顺序写入。
- Run summary metadata：为 `agent.listRuns` 设计并实现 `summary.json` 或等价索引，避免每次列出 run 都扫描完整 JSONL。
- RPC 全双工事件 writer 队列：当前 pending approval 已真实等待，但事件仍在 request 返回时 flush；后续让 `agent.sendTurn` 更早返回 accepted 并持续推送事件。
- RPC provider/tool 取消信号：当前 `agent.cancel` 已覆盖 pending approval，后续需要取消进行中的 provider request、工具进程和 client 断连场景。
- CLI JSON error response：让 `--json` 失败路径输出结构化 JSON-RPC error，而不是只写人类可读 stderr。
- 真实仓库验收：使用 DeepSeek provider 在小型仓库中执行一次“读取 -> 修改 -> 验证 -> 报告”。
- 测试替身收敛：如果 CLI fixture 场景继续增加，把 CLI fixture 与 Agent Core scripted provider 抽成共享测试 harness。

P0 不追求：

- 完整 TUI。
- 完整 VS Code Sidebar。
- TUI/VS Code 真实前端 UI 接入。
- MCP 生态。
- 多 provider UI。
- 大仓库 1M token 基准。

这些功能应在核心闭环可复现之后再推进。

## 跨阶段前置项

这些工作已提前实现，用于压低后续前端集成风险，但不作为 Phase 1 / Agent Core MVP 的阻塞验收条件：

- VS Code RPC server 管理：插件激活后可按配置启动 `deepseek-coder rpc`，发送 `agent.initialize`，转发 `agent.event`，并在进程退出或启动失败时提示用户；停止插件会关闭子进程。
- VS Code JSON-RPC request client：`RpcServerManager.sendRequest()` 统一管理 request id、pending response、error response 和进程退出时的 pending request 清理。

## P1：编辑器核心体验

目标：让 VS Code 插件成为 Agent Core 的薄前端，而不是第二套 Agent。

优先事项：

- 在已完成的 RPC server 管理和 request client 基础上，接入真实 Chat / Approval / Diff UI。
- Sidebar Chat 渲染 run events。
- 原生 diff editor 展示 patch。
- Problems 面板诊断进入 Context Builder。
- Terminal command approval 展示命令、cwd、风险等级和输出摘要。
- 命令风险分类器：识别网络访问、依赖安装、远程 git、发布和破坏性命令，并在审批前升级风险。
- Provider capability model：显式表达 thinking、tool choice、FIM、stream usage、cache usage、最大上下文和最大输出长度等能力。

验收重点：

- 插件和 CLI 对同一任务产生一致 run log。
- 插件不直接实现 tool execution、context builder 或 turn loop。

## P2：DeepSeek 差异化

目标：把 DeepSeek V4 的长上下文和思考模式变成可见、可审计的工作流。

优先事项：

- 1M Context Capsule。
- 稳定前缀与缓存命中统计。
- 真实 provider tokenizer 或经校准的 token estimator。
- `reasoning_content` replay 状态摘要。
- FIM completion preview。
- 高频 JSON-RPC streaming 性能基准和必要的 delta 合并策略。
- Run Log 轮转和导出体积控制。
- 小型真实仓库 benchmark，验证 1M 上下文、缓存布局和可审计 run log 是否带来可度量收益。

验收重点：

- 能解释哪些内容进入上下文、哪些没有进入，以及原因。
- 能记录 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` 并展示给前端。

## P3：生态扩展

目标：在核心闭环、编辑器体验和 DeepSeek 差异化稳定后，再扩展通用能力。

候选事项：

- MCP client。
- 本地模型或私有推理服务适配器。
- 包管理器工具。
- issue/PR 工具。
- 审计包导出。
- 可复现发布和多渠道安装。

## 风险控制

- 避免只堆文档和抽象：每个阶段都应有可运行验收场景。
- 避免前端行为分叉：CLI/TUI/VS Code 必须共享 Agent Core 和 run log。
- 避免过早追求通用多模型：provider API 要保持表达性，但 DeepSeek 是首个落地目标。
- 避免把 1M 上下文当作兜底：长上下文必须配合 token 预算、来源标注和验证命令。
