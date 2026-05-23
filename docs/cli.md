# CLI

状态：`0.1.0` Phase 1 `run` 最小闭环和 `--json` 实时事件输出已实现。

CLI 是最小可运行入口。它不重新实现 Agent Core，而是负责解析命令行参数、创建 run log、选择 provider、驱动 `AgentTurnLoop`，并把结果以人类可读摘要或 JSON-RPC event notification 输出。

## 当前命令

```powershell
deepseek-coder run [options] <task>
```

常用参数：

- `--workspace <path>`：workspace root，默认当前目录。
- `--provider <deepseek|fixture>`：provider 类型，默认 `deepseek`。
- `--fixture <final|readme|patch>`：fixture provider 的确定性脚本。
- `--mode <plan|edit|review|ask>`：Agent run mode。
- `--run-id <id>` / `--turn-id <id>`：显式指定本地 run/turn id。
- `--auto-approve` / `-y`：允许需要审批的工具执行。默认在 CLI 二进制中交互式询问；如果 stdin 已关闭或不可读，则拒绝该审批。
- `--verify <command>`：回合成功后运行显式验证命令。因为它执行 shell command，必须同时传 `--auto-approve`。
- `--json`：输出 newline-delimited `agent.event` JSON-RPC notifications。
- `--max-input-tokens <n>`、`--max-model-turns <n>`、`--max-output-tokens <n>`：预算与轮次限制。
- `--thinking <enabled|disabled>`：控制 DeepSeek thinking mode，默认 `enabled`。

## Provider

### `deepseek`

默认 provider。它读取 `DEEPSEEK_API_KEY`、`DEEPSEEK_BASE_URL` 和 `DEEPSEEK_MODEL`，通过现有 DeepSeek API adapter 发起 streaming chat completion。CLI 为整个 run 创建一个专用的 Tokio current-thread runtime，并启用 I/O 与 timer driver；当前 CLI 每次只驱动一个 run，这比 multi-thread runtime 更贴合串行命令行场景。

CLI DeepSeek provider 会把 streaming chunk 聚合成 Turn Loop 需要的完整 `Completed` 响应，同时把 content delta 转成 `AssistantDelta` 事件。`reasoning_content` delta 不作为用户可见输出写入 `assistant.delta`，只聚合后用于 thinking + tool calls 的 replay 校验。tool call delta 通过 adapter 的 `ChatToolCallAccumulator` 按 `index` 严格拼装：arguments 逐片追加，id、type 和 function name 必须在 stream 结束前出现且不能冲突。

当前 CLI provider 会把 executor 已实现的工具注册给模型：

- `read_file`
- `search`
- `apply_patch`
- `shell`
- `git_status`
- `git_diff`

`workspace_manifest`、`lsp_diagnostics`、`plan_update` 仍是 `schema_only`，不会暴露给 CLI 默认 provider。

### `fixture`

fixture provider 不联网，用于本地 smoke test 和 CI。它使用确定性的响应队列，每次 provider 请求弹出一条响应；脚本耗尽时显式失败，便于发现 Turn Loop 的异常重试或轮次配置问题。

- `final`：直接返回最终消息。
- `readme`：请求 `read_file README.md`，随后返回最终消息。
- `patch`：请求 `apply_patch` 修改 `CLI_SMOKE.txt`，随后返回最终消息。

`patch` fixture 会触发写入审批。交互式运行时可以直接输入 `y` 批准；自动化测试或非交互脚本可以使用 `--auto-approve`。

## 输出

默认输出人类可读摘要：

```text
runId: run_...
turnId: turn_1
events: <workspace>/.deepseek-coder/runs/<runId>/events.jsonl
status: completed
iterations: 2
tools: 1
final: ...
```

`--json` 输出与 RPC Server 相同的 `agent.event` notification，每行一条 JSON。事件在写入 run log 后立即输出，CLI 不再等 run 完成后批量回放：

```json
{"jsonrpc":"2.0","method":"agent.event","params":{"seq":1,"time":"...","type":"run.started","runId":"run_...","payload":{}}}
```

这让 CLI、TUI、VS Code 可以从同一份 run log 重建关键过程。输出顺序与本地 `events.jsonl` 的 `seq` 顺序一致；如果使用 streaming provider，`assistant.delta` 会在执行中持续出现。

## 验证命令

`--verify` 是显式用户命令，不由模型自动生成。CLI 会写入：

- `verification.started`
- `verification.completed`

命令失败时，CLI 返回非零错误，并保留 run log 事件。

`verification.completed` 中的 `stdout` / `stderr` 会在写入 run log 前经过与工具结果相同的脱敏处理。验证命令仍由用户显式提供；如果命令输出中包含 API key、token 或类似 `sk-...` 的密钥片段，本地事件流中只保留 `<redacted>`。

## 交互式审批

CLI 二进制默认会在 `apply_patch`、`shell` 等需要审批的工具执行前向 stderr 输出审批摘要，并从 stdin 读取 `y` / `n`。stdout 在 `--json` 模式下仍只保留 newline-delimited JSON-RPC 事件，便于前端或脚本解析；人类提示不会混入 stdout。

批准后，Run Log 会出现 `tool.approvalResolved`，随后进入 `tool.started`。拒绝后，Run Log 会出现 `tool.approvalResolved` 和 `run.failed`，对应工具不会执行。

## 当前限制

- `--json` 当前的失败路径仍输出人类可读错误；JSON-RPC error response 需要随 RPC request loop 一起设计。
- CLI DeepSeek provider 已能聚合 streaming tool call delta；但复杂工具调用的端到端 CLI live test 还没有覆盖真实写入、审批和继续请求。
- verification 只支持用户显式提供的单条 shell command。
- CLI 尚未接入 `agent-rpc` request loop；它直接调用 Agent Core，用于先完成本地最小闭环。
- 如果 fixture 场景继续增加，应把 CLI fixture 与 `agent-core` 的 scripted provider 抽成共享测试 harness，减少两套测试替身并行维护。

## 本地 smoke test 示例

无 API key 的确定性读文件：

```powershell
deepseek-coder run --provider fixture --fixture readme --mode ask "Read README"
```

确定性 patch + verification：

```powershell
Set-Content CLI_SMOKE.txt "old"
deepseek-coder run --provider fixture --fixture patch --auto-approve --verify "if ((Get-Content CLI_SMOKE.txt -Raw).Trim() -ne 'new') { exit 1 }" "Patch smoke file"
```

输出 JSON-RPC event notifications：

```powershell
deepseek-coder run --provider fixture --fixture readme --json "Read README"
```

仓库测试中已经包含进程级 fixture smoke test，会从编译出的 `deepseek-coder` 二进制启动，验证 `--json` 输出和 run log 都包含 `run.started`、`tool.completed` 和 `run.completed`。

真实 DeepSeek provider：

```powershell
deepseek-coder run --provider deepseek --mode ask "Summarize this workspace"
```

真实 DeepSeek streaming 验收：

```powershell
$env:DEEPSEEK_CODER_LIVE_TESTS = "1"
cargo test -p deepseek-coder-cli --test deepseek_cli_live live_deepseek_cli_streaming_smoke_test -- --ignored --exact --nocapture
```

该测试会从编译出的 `deepseek-coder` 二进制启动真实 `deepseek` provider，使用 streaming completion，并断言 JSON event 中存在 `stream: true` 的 `assistant.delta` 和最终 `run.completed`。模型默认使用项目默认的 `deepseek-v4-pro`；如果要临时改为其他模型，可以在当前 shell 设置 `DEEPSEEK_MODEL`。API Key 仍只来自当前环境变量或被忽略的 `.secrets/deepseek-api-key`。

当前已在 Windows 本机通过该 live smoke test。普通文本 streaming 由 CLI 二进制测试覆盖；真实 tool call delta 形态由 `agent-core` 的 `live_streaming_tool_call_accumulator_smoke_test` 覆盖。
