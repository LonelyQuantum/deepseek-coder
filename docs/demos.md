# 展示型 Demo

本文件集中记录所有用于“看过程和结果”的展示型 demo。它们服务于人工观察，不是默认 CI 的必跑项；新增 demo 时应同时更新本文件和 `.cargo/config.toml` 中的短命令。

## 运行约定

- 展示型 demo 默认使用 `#[ignore]`，避免被普通 `cargo test --workspace` 或 `pnpm run check` 自动执行。
- 推荐优先使用短命令，只有排查测试 harness 时才直接运行底层测试名。
- 真实联网 demo 必须显式设置 `DEEPSEEK_CODER_LIVE_TESTS=1`。
- 如果要保留临时工作区查看文件和 run log，先设置 `DEEPSEEK_CODER_KEEP_DEMO_WORKSPACE=1`。

```powershell
$env:DEEPSEEK_CODER_KEEP_DEMO_WORKSPACE = "1"
```

## Demo 清单

| Demo | 推荐命令 | 是否联网 | 用途 |
| --- | --- | --- | --- |
| Fixture Agent 交互转录 | `cargo demo` | 否 | 稳定展示工具调用、写入审批、补丁执行、验证命令和 run log 汇总。 |
| Live DeepSeek Agent 交互转录 | `cargo demo-live` | 是 | 使用真实 DeepSeek provider 展示读取文件、应用补丁、运行验证和最终总结。 |

## Fixture Agent 交互转录

该 demo 不需要 API key。它使用 fixture provider，在临时工作区中修改 `CLI_SMOKE.txt`，并打印：

- JSON-RPC event 的人类可读转录。
- 工具调用、审批结果、补丁执行和验证事件。
- 最终文件内容。
- `.deepseek-coder/runs/<run_id>/summary.json`。

推荐命令：

```powershell
cargo demo
```

底层测试：

```powershell
cargo test -p deepseek-coder-cli --test agent_interaction_demo fixture_agent_interaction_transcript_demo -- --ignored --exact --nocapture
```

## Live DeepSeek Agent 交互转录

该 demo 会调用真实 DeepSeek API。API key 来自当前环境变量 `DEEPSEEK_API_KEY`，或被 git 忽略的 `.secrets/deepseek-api-key`。它会在临时 Rust 小仓库中读取 `README.md` 和 `src/lib.rs`，让模型调用 `apply_patch` 修改代码，再由 harness 运行 `cargo test --quiet`。

推荐命令：

```powershell
$env:DEEPSEEK_CODER_LIVE_TESTS = "1"
cargo demo-live
```

底层测试：

```powershell
cargo test -p deepseek-coder-cli --test agent_interaction_demo live_deepseek_agent_interaction_transcript_demo -- --ignored --exact --nocapture
```

默认模型使用项目默认 DeepSeek 模型。需要临时改展示模型时，可以设置：

```powershell
$env:DEEPSEEK_AGENT_DEMO_MODEL = "deepseek-v4-pro"
```

## 新增 Demo 登记模板

新增展示型 demo 时，在上方清单增加一行，并补充一个同名小节：

```text
## <Demo 名称>

用途：
推荐命令：
底层测试：
是否联网：
预期输出：
注意事项：
```
