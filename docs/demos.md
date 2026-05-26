# 展示型 Demo

本文件集中记录所有用于“看过程和结果”的展示型 demo。它们服务于人工观察，不是默认 CI 的必跑项；新增 demo 时应同时更新本文件。只有 demo 已实现并可运行时，才同步把短命令加入 `.cargo/config.toml`，避免文档里出现尚不可用的命令。

## 运行约定

- 展示型 demo 默认使用 `#[ignore]`，避免被普通 `cargo test --workspace` 或 `pnpm run check` 自动执行。
- 推荐优先使用短命令，只有排查测试 harness 时才直接运行底层测试名。
- 真实联网 demo 必须显式设置 `DEEPSEEK_CODER_LIVE_TESTS=1`。
- 如果要保留临时工作区查看文件和 run log，先设置 `DEEPSEEK_CODER_KEEP_DEMO_WORKSPACE=1`。

```powershell
$env:DEEPSEEK_CODER_KEEP_DEMO_WORKSPACE = "1"
```

## 当前可运行 Demo

| Demo | 推荐命令 | 是否联网 | 用途 |
| --- | --- | --- | --- |
| Fixture Agent 交互转录 | `cargo demo` | 否 | 稳定展示工具调用、写入审批、补丁执行、验证命令和 run log 汇总。 |
| Context Capsule 结构展示 | `cargo demo-context` | 否 | 展示 manifest summary、Context Capsule sections、included/omitted sources 和 `context.built` payload。 |
| Run Log 截断展示 | `cargo demo-truncation` | 否 | 展示超大工具输出如何被脱敏、截断，并通过 `runLogTruncation` 区分截断、空输出和缺失字段。 |
| Tool Schema 错误展示 | `cargo demo-schema` | 否 | 展示模型 tool call arguments 在 typed deserialization 前被 JSON Schema 拒绝，并输出稳定 `E_INVALID_TOOL_ARGUMENTS`。 |
| Context Capsule ASCII 可视化 | `cargo demo-context-visual` | 否 | 用纯文本条形图展示 StablePrefix、DynamicPrelude、TurnSuffix token 分布，并输出原始 JSON。 |
| Attachment 上下文展示 | `cargo demo-attachment` | 否 | 展示 file、selection、explicit_content、diagnostic attachments 如何进入 Context Builder。 |
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

## Context Capsule 结构展示

该 demo 不需要 API key。它在临时工作区中构造一个小型项目，生成 workspace manifest 和 Context Capsule，并打印：

- manifest hash、文件数量、截断原因和 manifest entries。
- `StablePrefix`、`DynamicPrelude`、`TurnSuffix` 三段的 token 和条目。
- included/omitted sources。
- 原始 `context.built` payload。

推荐命令：

```powershell
cargo demo-context
```

## Context Capsule ASCII 可视化

该 demo 复用 Context Capsule fixture，在结构化输出之外额外打印一个纯文本 token 分布条，方便对照后续 VS Code Context Viz 的信息层级。

推荐命令：

```powershell
cargo demo-context-visual
```

## Run Log 截断展示

该 demo 不需要 API key。它向 run log 写入一个包含超大 `stdout` 和过多 `matches` 的工具结果，然后展示写入后的截断快照：

- `stdoutStoredBytes` 显示字符串存储边界。
- `matchesStored` 与 `matchesPreview` 区分截断数组和预览。
- `runLogTruncation` 记录每个被截断字段的 path、reason、original 和 stored。
- 空 `stderr` 与不存在的 `missingField` 会被明确区分。

推荐命令：

```powershell
cargo demo-truncation
```

## Tool Schema 错误展示

该 demo 不需要 API key。它使用 fixture provider 发出一个带未知字段的 `read_file` tool call，展示 tool arguments 会在 typed deserialization、审批和执行之前先被 JSON Schema 拒绝。

推荐命令：

```powershell
cargo demo-schema
```

预期输出包含 `E_INVALID_TOOL_ARGUMENTS`、`run.failed` 和 direct schema validator 的 path/detail。

## Attachment 上下文展示

该 demo 不需要 API key。它通过 `AgentTurnInput.attachments` 放入 file、selection、explicit_content 和 diagnostic 四类附件，展示它们如何进入 Context Builder 和最终 provider prompt。

推荐命令：

```powershell
cargo demo-attachment
```

预期输出包含 `context.built` 的六类来源，以及 prompt excerpt 中的 `Attachment-Kind: file`、`selection`、`explicit_content` 和 `diagnostic` 标记。

## Live DeepSeek Agent 交互转录

该 demo 会调用真实 DeepSeek API。API key 来自当前环境变量 `DEEPSEEK_CODER_API_KEY`、`DEEPSEEK_API_KEY`，或被 git 忽略的 `.secrets/deepseek-api-key`。它会在临时 Rust 小仓库中读取 `README.md` 和 `src/lib.rs`，让模型调用 `apply_patch` 修改代码，再由 harness 运行 `cargo test --quiet`。

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

输出摘要中应包含 `provider.completed`，展示模型、duration、usage、cache hit/miss 和 stream 统计字段；具体字段是否有数值取决于 provider 响应是否返回对应 usage/cache 数据。

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
