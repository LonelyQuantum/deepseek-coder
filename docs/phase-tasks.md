# 详细任务索引

状态：Phase 1 审计完成；后续任务已归入阶段。

本文档是详细设计文档里的任务账本。README 保留高层开发计划；这里把各模块文档中出现的“已实现、尚未实现、后续增强、下一步”收敛为可勾选任务，避免后续工作只散落在说明文字里。

维护规则：

- 新增任何预期实现项时，必须在本文件登记阶段和状态。
- README 开发计划中的阶段条目标记完成前，应检查本文件中对应细任务是否已经完成。
- 如果一个 README 条目完成了它蕴含的细任务，应同步把本文件对应行标记为 `[x]`，并在说明中写清验收方式。
- 详细模块文档仍保留设计说明；本文件只记录阶段、状态和追踪入口。

## 审计结论

本轮审计没有发现“详细文档声称属于 Phase 1 且已完成，但代码或测试明显尚未完成”的阻塞项。发现并修正了两处容易误导的描述：

- `docs/cli.md` 原先把 RPC 取消和审批过期也写成后续任务；实际 Phase 1 已实现显式取消、审批超时和 EOF shutdown 取消，未完成的是全双工后台事件 writer。
- `docs/architecture.md` 原先把 VS Code 插件描述成完全没有接入真实 RPC server；实际 RPC server 启动监管和 request client 已提前完成，未完成的是完整 Chat UI、事件渲染、审批回传和 diff editor 集成。

`RPC 全双工 reader/writer 与独立事件 writer 队列` 不属于 Phase 1 已完成范围；它现在明确归入 Phase 3 的共享 RPC 交互基础设施。

## Phase 0：项目章程

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [x] | 项目名称、AGPL-3.0-or-later、Rust/TypeScript workspace、pnpm workspace、Windows 环境说明、CI 骨架和治理文件 | `README.md`、`docs/adr/`、`CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md` | Phase 0 已完成并进入 README 高层计划。 |
| [x] | 建立 `docs/` 与 ADR，明确 README 只做入口和高层计划 | `docs/README.md`、`docs/adr/0005-keep-readme-as-entrypoint-and-move-design-to-docs.md` | 详细设计文档已按模块拆分。 |
| [x] | JSON-RPC 基础协议、工具 schema、风险等级和审批模型设计 | `docs/json-rpc-protocol.md`、`docs/tool-system.md`、`docs/approval-model.md` | Phase 0 设计完成；Phase 1 已实现基础执行闭环。 |

## Phase 1：Agent Core MVP

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [x] | DeepSeek API adapter、data-only SSE parser、streaming 基础和真实联网 smoke | `docs/deepseek-api-adapter.md`、`docs/agent-core.md` | 已有离线解析测试和 ignored live tests。 |
| [x] | streaming tool-call delta accumulator | `docs/deepseek-api-adapter.md`、`docs/roadmap.md` | 已覆盖 delta 拼装、冲突和缺失元数据测试，并有 live forced tool-call 验收。 |
| [x] | `reasoning_content` replay 状态机 | `docs/reasoning-content.md` | 已覆盖 replay required、缺失 reasoning、thinking disabled 等边界。 |
| [x] | 基础 Context Builder 与 token 预算报告 | `docs/context-capsule.md`、`docs/agent-core.md` | Phase 1 仅实现基础 builder；完整 1M Capsule 归入 Phase 2。 |
| [x] | read/search/apply_patch/shell/git 工具执行层 | `docs/tool-system.md` | 已覆盖路径约束、敏感路径拒绝、命令超时、结构化结果和工具取消。 |
| [x] | Run Log `events.jsonl`、`summary.json`、脱敏和写入串行化 | `docs/run-log.md` | 已接入 CLI 和 RPC；全双工 notification writer 不属于 Phase 1。 |
| [x] | Agent Turn Loop 基础编排、工具审批、验证命令和 run log 写入 | `docs/turn-loop.md`、`docs/agent-core.md` | 已有 fixture 端到端和 CLI smoke。 |
| [x] | `TurnProvider` async / streaming 边界和 `TurnEventSink` 实时事件出口 | `docs/turn-loop.md` | CLI `--json` 和 `StdioEventBridge` 已接入。 |
| [x] | CLI `run` / `rpc` 最小闭环和 JSON-RPC 错误输出 | `docs/cli.md` | 已有库级、进程级和 fixture smoke 测试。 |
| [x] | Agent RPC Server request loop、真实 Turn Loop handler、pending approval、取消、超时、EOF shutdown 和 `agent.listRuns` | `docs/rpc-server.md`、`docs/json-rpc-protocol.md` | 已覆盖审批批准/拒绝/取消/超时、并发拒绝、EOF shutdown、resume/listRuns。 |
| [x] | CLI/TUI/VS Code 审批前端基础原语 | `docs/approval-model.md`、`docs/tui.md`、`docs/vscode-extension.md` | CLI prompt、TUI prompt 状态机、VS Code modal adapter 已实现；完整 UI 接入归入后续阶段。 |
| [x] | Rust/TypeScript 工具注册表和错误码协议交叉校验 | `docs/tool-system.md`、`docs/json-rpc-protocol.md`、`packages/protocol` | 工具 registry fixture 与错误码表已进入默认测试。 |
| [x] | 合并前测试基础设施、live 配置收敛和离线最终验收 | `docs/testing.md`、`docs/demos.md` | `pnpm run check`、测试清单、离线 demo、diff/sensitive scan 已完成。 |
| [x] | VS Code RPC server 管理和 JSON-RPC request client 前置实现 | `docs/vscode-extension.md`、`docs/roadmap.md` | 属于 Phase 4 前置项，已提前完成；不作为 Phase 1 阻塞验收条件。 |

## Phase 2：1M Context Capsule

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [ ] | workspace manifest 自动构建 | `README.md`、`docs/context-capsule.md`、`docs/tool-system.md` | 包含文件清单、忽略规则、摘要和可审计来源。 |
| [ ] | 稳定前缀构建和缓存友好 prompt 布局 | `README.md`、`docs/context-capsule.md`、`docs/deepseek-api-adapter.md` | 面向 DeepSeek 1M 长上下文和 prompt cache。 |
| [ ] | 真实 provider tokenizer 或校准后的 token estimator | `docs/roadmap.md`、`docs/context-capsule.md` | 当前 `utf8_bytes` 是确定性代理估算。 |
| [ ] | token 预算报告扩展到 200K、500K、900K 样例仓库 | `README.md`、`docs/testing.md`、`docs/context-capsule.md` | 需要 ignored/manual benchmark，不能默认进入 CI。 |
| [ ] | Context Builder 接入选中文件、诊断、工具结果、计划步骤和显式 attachment | `docs/json-rpc-protocol.md`、`docs/context-capsule.md` | `agent.sendTurn.attachments` 当前会被拒绝。 |
| [ ] | 缓存命中统计与 provider summary 事件 | `README.md`、`docs/roadmap.md`、`docs/deepseek-api-adapter.md` | 记录模型名、usage、cache hit/miss、stream 统计等稳定 schema。 |
| [ ] | 超预算停止机制和上下文省略原因展示 | `README.md`、`docs/context-capsule.md` | 当前已有基础 required/optional budget 行为；Phase 2 需要面向用户可解释。 |
| [ ] | Run Log 体积、输出截断和脱敏包边界 | `docs/run-log.md`、`docs/roadmap.md`、`docs/security-model.md` | 包含工具输出、verification 输出和 provider 摘要的统一大小限制。 |
| [ ] | `read_file` / manifest 内容摘要字段，例如 `sha256` | `docs/tool-system.md` | 当前 Phase 1 `read_file` 不返回内容摘要。 |
| [ ] | tool call JSON Schema 通用校验层 | `docs/agent-core.md`、`docs/tool-system.md` | 当前主要依赖结构化参数反序列化和工具定义；后续补统一 schema validator。 |

## Phase 3：TUI 与共享 RPC 交互管线

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [ ] | RPC 全双工 reader/writer 与独立事件 writer 队列 | `docs/rpc-server.md`、`docs/turn-loop.md`、`docs/run-log.md`、`docs/roadmap.md` | 让 `agent.sendTurn` 更早返回 accepted，后台持续推送事件，并保证同一 run notification 按 `seq` 串行。 |
| [ ] | 长 provider request 期间的 client 断连取消 | `docs/rpc-server.md`、`docs/json-rpc-protocol.md`、`docs/approval-model.md` | Phase 1 已支持 pending approval EOF shutdown；这里扩展到全双工运行中的断连感知。 |
| [ ] | TUI RPC 入口和事件流消费 | `README.md`、`docs/tui.md` | 消费 `agent.event`，展示 run、turn、工具和审批状态。 |
| [ ] | TUI Chat / Plan / Diff / Tools / Context / Settings 页面 | `README.md`、`docs/tui.md` | 完整 ratatui 界面仍未实现。 |
| [ ] | TUI hunk 级审批、run resume、配置文件和 release binary | `README.md`、`docs/tui.md` | 建议在全双工事件管线稳定后推进。 |
| [ ] | 命令风险分类器和动态风险升级 | `docs/approval-model.md`、`docs/tool-system.md`、`docs/turn-loop.md` | 识别依赖安装、网络访问、远程 git、删除和发布命令。 |
| [ ] | 更强进程树清理策略 | `docs/tool-system.md`、`docs/security-model.md`、`docs/roadmap.md` | 当前 shell/search/git 等只做基础协作式取消和 child kill。 |
| [ ] | 展示型 demo 扩展到 RPC 审批、JSON 错误、run list/resume | `docs/demos.md`、`docs/roadmap.md` | 统一登记到 `docs/demos.md`，默认 ignored。 |

## Phase 4：VS Code 插件

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [x] | TypeScript extension scaffold | `README.md`、`docs/vscode-extension.md` | 已完成基础命令和测试骨架。 |
| [x] | RPC server 启动监管 | `README.md`、`docs/vscode-extension.md` | 已能启动 `deepseek-coder rpc`、发送 initialize、转发事件并处理退出。 |
| [x] | JSON-RPC request client | `README.md`、`docs/vscode-extension.md` | 已管理 request id、pending response、error response 和进程退出清理。 |
| [ ] | Sidebar Chat 和 `agent.event` 渲染 | `README.md`、`docs/vscode-extension.md` | 当前 manager 能转发事件，但 UI 尚未消费。 |
| [ ] | VS Code 审批 UI 接入真实 RPC pending queue | `docs/approval-model.md`、`docs/vscode-extension.md` | modal adapter 已有，仍需消费 `tool.approvalRequired` 并发送 approve/reject。 |
| [ ] | Native diff editor 与 hunk 级审批 | `README.md`、`docs/vscode-extension.md` | 需要和 patch/apply result、审批模型联动。 |
| [ ] | Problems 面板诊断进入 Context Builder | `README.md`、`docs/vscode-extension.md`、`docs/context-capsule.md` | 依赖 Phase 2 attachment/context 输入稳定。 |
| [ ] | Terminal command approval | `README.md`、`docs/vscode-extension.md`、`docs/approval-model.md` | 展示命令、cwd、风险等级、输出摘要和持久化选项。 |
| [ ] | FIM completion preview | `README.md`、`docs/deepseek-api-adapter.md`、`docs/vscode-extension.md` | 需要 provider capability model 与编辑器 UI。 |
| [ ] | Provider capability model | `docs/roadmap.md`、`docs/deepseek-api-adapter.md` | 显式表达 thinking、tool choice、FIM、stream usage、cache usage、上下文和输出限制。 |

## Phase 5：发布与生态

| 状态 | 任务 | 来源 | 说明 |
| --- | --- | --- | --- |
| [x] | 许可证策略确定为 AGPL-3.0-or-later | `README.md`、`docs/release.md`、`docs/adr/0003-use-agpl-3.0-or-later.md` | 正式发布文件仍在后续任务。 |
| [ ] | 发布 `LICENSE`、源码获取说明和网络服务源码提供说明 | `README.md`、`docs/release.md` | 发布前必需。 |
| [ ] | 发布源码包、Cargo crate、npm wrapper、VSIX、GitHub Release 校验和 | `README.md`、`docs/release.md` | 需要发布脚本和产物签名/校验策略。 |
| [ ] | 公开 roadmap、issue 模板和贡献流程增强 | `README.md`、`CONTRIBUTING.md` | 面向外部协作者。 |
| [ ] | reproducible build 说明 | `README.md`、`docs/release.md` | 发布可信度要求。 |
| [ ] | MCP client、本地模型/私有推理服务 adapter、包管理器工具、issue/PR 工具、审计包导出 | `docs/roadmap.md` | 生态扩展应在核心闭环、编辑器体验和 DeepSeek 差异化稳定后推进。 |
