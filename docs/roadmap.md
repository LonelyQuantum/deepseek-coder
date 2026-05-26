# 路线图

状态：草案，Phase 1 Agent Core MVP、合并主线前离线最终验收和 Phase 2 的 1M Context Capsule 核心收敛已完成；Phase 2 合并主线前还需要完成展示型 demo 扩展，之后随 Phase 3 VS Code 插件核心与共享 RPC 事件队列实现持续更新。

本文档把 README 中的大阶段拆成更可执行的优先级。README 保留项目入口和高层计划；这里记录跨模块的落地顺序、取舍和验收重点。具体任务的阶段、状态和来源统一登记在 `docs/phase-tasks.md`，阶段条目标记完成前应同步检查并更新该索引。

## 定位

`deepseek-coder` 不以第一阶段覆盖通用 AI 编程工具的全部功能为目标。项目的核心差异是：

- DeepSeek V4 1M 上下文优先。
- `reasoning_content`、上下文缓存和长输出能力一等支持。
- Rust Agent Core 供 CLI/TUI/VS Code 共用。
- 可复现 run log、显式审批和自由软件治理。
- 中文工作流友好。

因此短期不和成熟 VS Code 扩展正面拼功能覆盖率，而是先做出一个可审计、可复现、能稳定闭环的小型 Agent。

## P0：Phase 1 MVP 与合并主线前收敛

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
- 进程级 CLI fixture smoke test：从编译出的 `deepseek-coder` 二进制启动，验证 CLI、Turn Loop、Run Log、JSON-RPC event 输出、事件序号连续性和关键事件顺序的最小闭环。
- TurnProvider async / streaming 边界：`TurnProvider::complete_stream` 返回异步事件流，支持 `assistant.delta` 与最终 `Completed` 响应。
- CLI DeepSeek provider streaming wrapper：CLI provider 通过 `create_chat_completion_stream` 聚合 content、`reasoning_content` 和 tool calls，并把 content delta 写入 run log。
- 真实 provider streaming 联网验收：`deepseek_cli_live` 从编译出的 CLI 二进制启动真实 DeepSeek provider，验证 `stream: true` 的 `assistant.delta` 和最终 `run.completed`。
- streaming tool call 增量拼装验证：adapter 已区分 `ChatToolCallDelta` 与完整 `ChatToolCall`，`ChatToolCallAccumulator` 会按 `index` 拼接 arguments 并拒绝缺失或冲突元数据；`live_streaming_tool_call_accumulator_smoke_test` 已用真实 DeepSeek streaming 验收工具调用 delta 形态。
- Agent RPC Server 双向 request loop：`agent-rpc` 已支持 newline-delimited JSON-RPC request 读取、初始化顺序检查、`agent.initialize` / `agent.sendTurn` / `agent.resume` 分发、response/error 写回、EOF shutdown，以及 handler 返回事件的 `agent.event` 有序输出。
- RPC/CLI 实时事件输出：`AgentTurnLoop::run_turn_with_event_sink` 会在 Run Log 事件追加成功后立即调用 `TurnEventSink`；`StdioEventBridge` 已实现该接口，CLI `--json` 输出顺序与本地 `events.jsonl` 的 `seq` 一致，不再等 run 完成后批量回放。
- CLI/RPC/TUI/VS Code 审批基础：Turn Loop 会写入 `tool.approvalRequired` 和 `tool.approvalResolved`；CLI 二进制支持 stdin/stderr 交互式 y/n 审批；RPC request loop 已分发 `agent.approve` / `agent.reject`；TypeScript 协议类型已补齐；TUI prompt 状态机和 VS Code modal approval adapter 已有测试覆盖。
- 真实 RPC Turn Loop handler：`AgentTurnLoopRpcHandler` 已通过 provider factory 复用 Core Turn Loop，`agent.sendTurn` 会创建 run log、驱动 provider 和工具执行，并把结果事件交给 request loop；CLI `rpc` 子命令已提供 stdio 入口。
- RPC 真实审批等待队列：`AgentTurnLoopRpcHandler` 会在 `tool.approvalRequired` 处登记 pending approval，后台 Turn Loop worker 等待 `agent.approve` / `agent.reject`，批准后继续执行工具，拒绝后记录 `tool.approvalResolved` 和 `run.failed`。
- RPC 审批超时/取消：pending approval 已记录过期时间；`agent.cancel` 和 request loop EOF shutdown 会取消等待审批的 active run，超时会自动解析为 expired，这些路径都会记录 `tool.approvalResolved` 和 `run.canceled`。
- Run Log 写入串行化：Agent Core 已提供 `RunLogWriter` trait 和 `SerializedRunLog`；CLI 继续使用单 writer `RunLog`，RPC active run 使用共享锁串行化 append/load，避免同一 run 被多个前端请求并发读写时出现序列错乱。
- Run summary metadata / `agent.listRuns`：每个 run 目录维护 `summary.json`，记录标题、状态、时间、事件数、最终摘要、变更文件和验证状态；RPC `agent.listRuns` 通过 summary 快速列出 run。
- RPC provider/tool 取消信号：Agent Core 已提供协作式 `CancellationToken`；RPC active run 会把 token 注入 Turn Loop，`agent.cancel` 会通知 provider request、命令类工具和 pending approval，并以 `run.canceled` 收口。
- CLI JSON error response：`run --json` 失败路径会在 stdout 输出 JSON-RPC error response，保留非零退出码，并避免把人类错误摘要混入 stdout。
- 小型真实仓库 CLI 验收：`live_deepseek_cli_real_repo_acceptance_test` 已通过，真实 DeepSeek provider 在临时 Rust 仓库中完成“读取 -> 修改 -> 验证 -> 报告”。

合并主线前已完成：

- `pnpm run check` 基线验证：Windows 本机已通过默认 CI 等价检查。
- Context Builder token 预算测试：当前已覆盖 token 报告、可选上下文超预算省略、必需上下文超预算失败和 `context.built` payload 形状。
- Patch apply 失败恢复：`apply_patch` 已改为先 staging 再写盘，并有多文件失败不留半修改的回归测试。
- `reasoning_content` 状态机边界：已覆盖空消息、多个 tool-call assistant message 和 replay 计数。
- `CancellationToken` 并发语义：已覆盖 clone 共享状态、首次取消原因保持和并发取消。
- CLI event stream 顺序：进程级 smoke test 已验证 event `seq` 连续递增和关键事件子序列。
- 共享 `TestWorkspace`：`agent-core::test_helpers::TestWorkspace` 已统一 agent-core、agent-rpc、cli、demo/live 测试的临时工作区创建、保留、git 初始化和读写 helper。
- live API key 测试 helper：真实联网测试已统一通过 `DEEPSEEK_CODER_API_KEY -> DEEPSEEK_API_KEY -> .secrets/deepseek-api-key` 读取本地密钥；运行时 provider 配置保持不变。
- RPC/CLI/protocol 合并前验收：`agent-rpc` 新增 pending approval 并发拒绝与 EOF shutdown 取消测试，`cli` 新增真实二进制 `rpc` stdio smoke，`packages/protocol` 与 `agent-rpc` 共同校验错误码表和协议文档一致。

合并主线前最终验收：

- 已运行 `pnpm run check`、`cargo test --workspace -- --list`、离线展示 demo、`git diff --check` 和敏感信息扫描。
- 本轮 RPC/CLI/protocol 离线变更未新增必须阻塞合并的 DeepSeek live suite；已有 live suite 仍按 `docs/testing.md` 保留为阶段合并前的手动验收选项。

下一步：

- 完成 Phase 2e 展示型 demo 扩展，作为 Phase 2 合并主线前验收。
- 随后进入 Phase 3 VS Code 插件核心与共享 RPC 交互管线。
- RPC 全双工事件 writer 队列已归入 Phase 3 共享 RPC 交互管线；当前 pending approval 已真实等待，但事件仍在 request 返回时 flush，后续让 `agent.sendTurn` 更早返回 accepted 并持续推送事件。
- TUI 保留为正式前端，但优先级调整到 VS Code 核心体验之后，复用同一套 RPC 事件管线和审批模型。

Phase 1 收官后优化池：

- RPC 事件输出模型收敛：把当前 request 边界 flush 的事件输出升级为独立 writer 队列，把 EOF shutdown 的 pending approval 取消扩展为长 provider request 期间也能即时感知 client 断连。
- 工具执行安全打磨：实现命令风险分类器，补充进程树清理策略，并在审批信息中突出 cwd、命令摘要和风险升级原因。
- Run Log 体积与隐私控制：为工具输出、verification 输出和 provider 摘要增加统一大小限制、截断原因和可导出的脱敏包边界。
- Provider summary 事件：把 usage、cache 命中、模型名、stream 统计等写成稳定 schema，避免只依赖 provider 私有响应。
- 本地环境诊断：增加 doctor 类检查，显式验证 `rg`、`git`、`cargo`、Node/pnpm、API key 来源和 workspace 信任状态；不做隐式搜索 backend 降级。
- 展示型 demo 扩展：在已有 `cargo demo` / `cargo demo-live` 基础上，于 Phase 2 合并主线前补齐 context、truncation、schema、context-visual、attachment 和 provider summary 等展示场景，统一登记到 `docs/demos.md`。

P0 不追求：

- 完整 VS Code Sidebar：已移入 Phase 3。
- 完整 TUI：已移入 Phase 5。
- VS Code/TUI 真实前端 UI 接入：Phase 3 优先 VS Code，Phase 5 再补齐 TUI。
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

Phase 3 优先交付 VS Code 插件核心体验；Phase 4 再做 VS Code 深度集成；TUI 进入 Phase 5，与生态扩展一起推进。Marketplace 发布不阻塞 Phase 3 完成，先保证本地可安装、可运行、可审计。Phase 3 开始前，Phase 2e 应先完成展示型 demo 扩展，给 VS Code Context Viz / Approval / Run Log UI 提供可观察样本。

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

Phase 2 的 1M Context Capsule 按 4 个增量轮次推进：

1. **Phase 2a：Context Capsule 数据模型与 Manifest v0**
   - [x] `read_file` 增加 `sha256` / `sizeBytes`。
   - [x] 定义 `ContextCapsule`、`ContextSection`、`CachePlacement` 和稳定 renderer。
   - [x] 实现 workspace manifest v0：结构化 JSON、canonical `manifestHash`、默认 `maxEntries=500`、硬安全排除、默认工程排除、`.gitignore` + `.deepseek-coderignore`。
   - [x] Context Builder 接入 manifest summary，并扩展 `context.built` payload。

2. **Phase 2b：TokenEstimator 与稳定前缀**
   - [x] 建立 `TokenEstimator` trait，保留 `utf8_bytes` 默认估算器。
   - [x] 增加基于 provider usage 样本的 `CalibratedEstimator`，但仍标注 `exact=false`，且不保存 prompt 原文。
   - [x] 按 `CachePlacement::{StablePrefix, DynamicPrelude, TurnSuffix}` 构建缓存友好 prompt，并输出 `stablePrefixHash` 与稳定前缀预算。

3. **Phase 2c：Attachments、provider summary 与 cache 实验**
   - [x] 接入 `agent.sendTurn.attachments` 的 file、selection/explicit_content、diagnostic。
   - [x] 新增 `provider.completed` 事件，记录模型、duration、usage、cache hit/miss 和 stream 摘要。
   - [x] 建立 DeepSeek cache hit/miss ignored live experiment 的基础解析路径；更大重复前缀样本归入 Phase 2d 前增强。

4. **Phase 2d：大仓库验收与体积控制**
   - [x] 200K、500K、900K 样例仓库 token 预算和 Context Capsule ignored/manual 验收。
   - [x] 超预算解释、Run Log 输出截断和脱敏包边界。
   - [x] tool call JSON Schema 通用校验层，且在 typed deserialization 前执行。

5. **Phase 2e：合并主线前展示型 demo 扩展**
   - [ ] `demo-context`：展示 manifest summary、Context Capsule sections、included/omitted sources 和 `context.built` payload。
   - [ ] `demo-truncation`：展示 Run Log 脱敏、截断、`runLogTruncation`，并区分截断、空输出和缺失字段。
   - [ ] `demo-schema`：展示 tool call arguments 在 typed deserialization 前被 JSON Schema 拒绝。
   - [ ] `demo-context-visual`：用 ASCII 视图展示 StablePrefix、DynamicPrelude、TurnSuffix 的 token 分布，并输出原始 JSON。
   - [ ] `demo-attachment`：展示 file、selection、explicit_content、diagnostic attachments 如何进入 Context Builder。
   - [ ] `demo-live` provider summary 增强：展示模型、duration、usage、cache hit/miss 和 stream 摘要。

后续 DeepSeek 差异化事项：

- `reasoning_content` replay 状态摘要。
- FIM completion preview。
- 高频 JSON-RPC streaming 性能基准和必要的 delta 合并策略。

验收重点：

- 能解释哪些内容进入上下文、哪些没有进入，以及原因。
- 能记录 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` 并展示给前端。
- 同一输入两次构建的 `StablePrefix` 渲染完全一致，修改 `TurnSuffix` 不影响稳定前缀。
- Manifest 的 ignore、sha256、manifest hash、截断和 omitted reason 均可离线测试。

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
