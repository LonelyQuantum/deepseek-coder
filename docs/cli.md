# CLI

状态：`0.1.0` Phase 1 `run` 最小闭环已实现。

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
- `--auto-approve` / `-y`：允许需要审批的工具执行。默认拒绝写入和命令。
- `--verify <command>`：回合成功后运行显式验证命令。因为它执行 shell command，必须同时传 `--auto-approve`。
- `--json`：输出 newline-delimited `agent.event` JSON-RPC notifications。
- `--max-input-tokens <n>`、`--max-model-turns <n>`、`--max-output-tokens <n>`：预算与轮次限制。

## Provider

### `deepseek`

默认 provider。它读取 `DEEPSEEK_API_KEY`、`DEEPSEEK_BASE_URL` 和 `DEEPSEEK_MODEL`，通过现有 DeepSeek API adapter 发起非 streaming chat completion。CLI 为 provider 创建一个专用的 Tokio multi-thread runtime，并启用 I/O 与 timer driver，避免把 HTTP 调用和主线程输出逻辑绑在同一个执行上下文里。

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

`patch` fixture 需要 `--auto-approve`，因为它触发写入审批。

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

`--json` 输出与 RPC Server 相同的 `agent.event` notification，每行一条 JSON：

```json
{"jsonrpc":"2.0","method":"agent.event","params":{"seq":1,"time":"...","type":"run.started","runId":"run_...","payload":{}}}
```

这让 CLI、TUI、VS Code 可以从同一份 run log 重建关键过程。

## 验证命令

`--verify` 是显式用户命令，不由模型自动生成。CLI 会写入：

- `verification.started`
- `verification.completed`

命令失败时，CLI 返回非零错误，并保留 run log 事件。

`verification.completed` 中的 `stdout` / `stderr` 会在写入 run log 前经过与工具结果相同的脱敏处理。验证命令仍由用户显式提供；如果命令输出中包含 API key、token 或类似 `sk-...` 的密钥片段，本地事件流中只保留 `<redacted>`。

## 当前限制

- DeepSeek provider 当前使用非 streaming completion；streaming delta 到 `assistant.delta` 的实时映射仍属于后续工作。
- CLI 没有交互式审批 UI；默认拒绝写入/命令，`--auto-approve` 是显式非交互模式。
- `--json` 当前在 run 完成后从 run log 重放事件，而不是边执行边实时输出。
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

真实 DeepSeek provider：

```powershell
deepseek-coder run --provider deepseek --mode ask "Summarize this workspace"
```
