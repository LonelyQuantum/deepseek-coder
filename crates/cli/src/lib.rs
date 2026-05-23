#![forbid(unsafe_code)]

use std::{
    collections::VecDeque,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use deepseek_coder_agent_core::{
    AGENT_METADATA,
    provider::deepseek_api::{
        ChatFunctionDefinition, ChatTool, ChatToolCall, DeepSeekApiAdapter, DeepSeekApiConfig,
    },
    run_log::{RunLog, RunLogError, RunLogStore},
    tool::{BUILTIN_TOOLS, ToolImplementationStatus},
    tool_execution::{
        ShellArgs, ToolExecutionError, ToolStatus, WorkspaceToolExecutor,
        redacted_tool_result_value,
    },
    turn_loop::{
        AgentRunMode, AgentTurnInput, AgentTurnLoop, AgentTurnLoopConfig, AgentTurnLoopError,
        AgentTurnOutcome, ApprovalPolicy, AutoApprovePolicy, RejectAllApprovalPolicy, TurnProvider,
        TurnProviderError, TurnProviderRequest, TurnProviderResponse,
    },
};
use deepseek_coder_agent_rpc::{AgentRpcError, StdioEventBridge};
use serde_json::json;
use thiserror::Error;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_024;
const DEFAULT_VERIFY_TIMEOUT_MS: u64 = 120_000;

pub fn run_cli<I, S, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
    E: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let command = CliCommand::parse(&args)?;

    match command {
        CliCommand::Help => {
            writeln!(stdout, "{}", help_text())?;
            Ok(())
        }
        CliCommand::Run(command) => run_command(command, stdout, stderr),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Help,
    Run(RunCommand),
}

impl CliCommand {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut args = args.iter().skip(1);
        let Some(command) = args.next() else {
            return Ok(Self::Help);
        };

        match command.as_str() {
            "-h" | "--help" | "help" => Ok(Self::Help),
            "run" => Ok(Self::Run(RunCommand::parse(args.cloned().collect())?)),
            other => Err(CliError::Usage(format!(
                "unknown command `{other}`\n\n{}",
                help_text()
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunCommand {
    task: String,
    workspace: PathBuf,
    run_id: String,
    turn_id: String,
    mode: AgentRunMode,
    provider: ProviderKind,
    fixture: FixtureKind,
    auto_approve: bool,
    json_events: bool,
    verify_command: Option<String>,
    verify_timeout_ms: u64,
    max_input_tokens: u64,
    max_model_turns: usize,
    max_output_tokens: u32,
}

impl RunCommand {
    fn parse(args: Vec<String>) -> Result<Self, CliError> {
        let mut workspace = None;
        let mut run_id = None;
        let mut turn_id = None;
        let mut mode = AgentRunMode::Edit;
        let mut provider = ProviderKind::DeepSeek;
        let mut fixture = FixtureKind::Readme;
        let mut auto_approve = false;
        let mut json_events = false;
        let mut verify_command = None;
        let mut verify_timeout_ms = DEFAULT_VERIFY_TIMEOUT_MS;
        let mut max_input_tokens = AgentTurnLoopConfig::default().max_input_tokens;
        let mut max_model_turns = AgentTurnLoopConfig::default().max_model_turns;
        let mut max_output_tokens = DEFAULT_MAX_OUTPUT_TOKENS;
        let mut task_parts = Vec::new();
        let mut index = 0;

        while index < args.len() {
            let arg = &args[index];
            match arg.as_str() {
                "-h" | "--help" => {
                    return Err(CliError::Usage(run_help_text()));
                }
                "--workspace" => {
                    workspace = Some(PathBuf::from(next_value(&args, &mut index, "--workspace")?));
                }
                "--run-id" => {
                    run_id = Some(next_value(&args, &mut index, "--run-id")?);
                }
                "--turn-id" => {
                    turn_id = Some(next_value(&args, &mut index, "--turn-id")?);
                }
                "--mode" => {
                    mode = parse_mode(&next_value(&args, &mut index, "--mode")?)?;
                }
                "--provider" => {
                    provider = parse_provider(&next_value(&args, &mut index, "--provider")?)?;
                }
                "--fixture" => {
                    fixture = parse_fixture(&next_value(&args, &mut index, "--fixture")?)?;
                }
                "--auto-approve" | "-y" => {
                    auto_approve = true;
                }
                "--json" => {
                    json_events = true;
                }
                "--verify" => {
                    verify_command = Some(next_value(&args, &mut index, "--verify")?);
                }
                "--verify-timeout-ms" => {
                    verify_timeout_ms =
                        parse_u64(&next_value(&args, &mut index, "--verify-timeout-ms")?)?;
                }
                "--max-input-tokens" => {
                    max_input_tokens =
                        parse_u64(&next_value(&args, &mut index, "--max-input-tokens")?)?;
                }
                "--max-model-turns" => {
                    max_model_turns =
                        parse_usize(&next_value(&args, &mut index, "--max-model-turns")?)?;
                }
                "--max-output-tokens" => {
                    max_output_tokens =
                        parse_u32(&next_value(&args, &mut index, "--max-output-tokens")?)?;
                }
                "--" => {
                    task_parts.extend(args[index + 1..].iter().cloned());
                    break;
                }
                value if value.starts_with('-') => {
                    return Err(CliError::Usage(format!("unknown run option `{value}`")));
                }
                value => task_parts.push(value.to_owned()),
            }

            index += 1;
        }

        let task = task_parts.join(" ");
        if task.trim().is_empty() {
            return Err(CliError::Usage("run requires a task message".to_owned()));
        }

        if verify_command.is_some() && !auto_approve {
            return Err(CliError::Usage(
                "--verify executes a command and requires --auto-approve".to_owned(),
            ));
        }

        let generated_run_id = generate_id("run")?;
        Ok(Self {
            task,
            workspace: workspace.unwrap_or(env::current_dir()?),
            run_id: run_id.unwrap_or(generated_run_id),
            turn_id: turn_id.unwrap_or_else(|| "turn_1".to_owned()),
            mode,
            provider,
            fixture,
            auto_approve,
            json_events,
            verify_command,
            verify_timeout_ms,
            max_input_tokens,
            max_model_turns,
            max_output_tokens,
        })
    }
}

fn run_command<W, E>(command: RunCommand, stdout: &mut W, stderr: &mut E) -> Result<(), CliError>
where
    W: Write,
    E: Write,
{
    let workspace = fs::canonicalize(&command.workspace).map_err(|source| CliError::Io {
        path: command.workspace.clone(),
        source,
    })?;
    let store = RunLogStore::new(&workspace)?;
    let mut run_log = store.create_run(command.run_id.clone())?;
    let provider = create_provider(&command)?;
    let config = AgentTurnLoopConfig {
        max_input_tokens: command.max_input_tokens,
        max_model_turns: command.max_model_turns,
    };
    let input =
        AgentTurnInput::new(command.turn_id.clone(), command.task.clone()).with_mode(command.mode);

    let turn_result = if command.auto_approve {
        run_with_policy(
            &workspace,
            provider,
            AutoApprovePolicy,
            config,
            input,
            &mut run_log,
        )
    } else {
        run_with_policy(
            &workspace,
            provider,
            RejectAllApprovalPolicy,
            config,
            input,
            &mut run_log,
        )
    };

    let verification_result = match (&turn_result, &command.verify_command) {
        (Ok(_), Some(verify_command)) => run_verification(
            &workspace,
            &command.turn_id,
            verify_command,
            command.verify_timeout_ms,
            &mut run_log,
        )
        .map(Some),
        _ => Ok(None),
    };

    let events = run_log.load()?;
    if command.json_events {
        let mut bridge = StdioEventBridge::new(stdout);
        bridge.emit_events(&events)?;
    } else {
        emit_human_summary(
            &command,
            run_log.events_path(),
            &turn_result,
            verification_result.as_ref().ok().and_then(Option::as_ref),
            stdout,
            stderr,
        )?;
    }

    let outcome = turn_result?;
    verification_result?;

    if outcome.final_message.trim().is_empty() {
        return Err(CliError::EmptyFinalMessage);
    }

    Ok(())
}

fn run_with_policy<A>(
    workspace: &Path,
    provider: CliTurnProvider,
    approval_policy: A,
    config: AgentTurnLoopConfig,
    input: AgentTurnInput,
    run_log: &mut RunLog,
) -> Result<AgentTurnOutcome, AgentTurnLoopError>
where
    A: ApprovalPolicy,
{
    let mut loop_runner =
        AgentTurnLoop::with_approval_policy(workspace, provider, approval_policy)?
            .with_config(config);
    loop_runner.run_turn(input, run_log)
}

fn run_verification(
    workspace: &Path,
    turn_id: &str,
    command: &str,
    timeout_ms: u64,
    run_log: &mut RunLog,
) -> Result<VerificationOutcome, CliError> {
    let verification_id = "verification_1";
    run_log.append(
        "verification.started",
        Some(turn_id.to_owned()),
        json!({
            "verificationId": verification_id,
            "command": command,
            "cwd": ".",
        }),
    )?;

    let tools = WorkspaceToolExecutor::new(workspace)?;
    let result = tools.shell(ShellArgs {
        command: command.to_owned(),
        cwd: None,
        timeout_ms: Some(timeout_ms),
    })?;
    let status = match result.status {
        ToolStatus::Ok => "passed",
        ToolStatus::Failed => "failed",
    };
    let redacted_result = redacted_tool_result_value(&result)?;

    run_log.append(
        "verification.completed",
        Some(turn_id.to_owned()),
        json!({
            "verificationId": verification_id,
            "status": status,
            "exitCode": result.exit_code,
            "stdout": redacted_result["stdout"].clone(),
            "stderr": redacted_result["stderr"].clone(),
            "durationMs": result.duration_ms,
        }),
    )?;

    let outcome = VerificationOutcome {
        status: result.status,
        exit_code: result.exit_code,
    };
    if result.status == ToolStatus::Failed {
        return Err(CliError::VerificationFailed {
            exit_code: result.exit_code,
        });
    }

    Ok(outcome)
}

fn emit_human_summary<W, E>(
    command: &RunCommand,
    events_path: &Path,
    turn_result: &Result<AgentTurnOutcome, AgentTurnLoopError>,
    verification: Option<&VerificationOutcome>,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError>
where
    W: Write,
    E: Write,
{
    writeln!(stdout, "runId: {}", command.run_id)?;
    writeln!(stdout, "turnId: {}", command.turn_id)?;
    writeln!(stdout, "events: {}", events_path.display())?;

    match turn_result {
        Ok(outcome) => {
            writeln!(stdout, "status: completed")?;
            writeln!(stdout, "iterations: {}", outcome.iterations)?;
            writeln!(stdout, "tools: {}", outcome.tool_results.len())?;
            if !outcome.changed_files.is_empty() {
                writeln!(stdout, "changedFiles: {}", outcome.changed_files.join(", "))?;
            }
            if let Some(verification) = verification {
                writeln!(
                    stdout,
                    "verification: {}",
                    match verification.status {
                        ToolStatus::Ok => "passed",
                        ToolStatus::Failed => "failed",
                    }
                )?;
                if let Some(exit_code) = verification.exit_code {
                    writeln!(stdout, "verificationExitCode: {exit_code}")?;
                }
            }
            writeln!(stdout, "final: {}", outcome.final_message)?;
        }
        Err(error) => {
            writeln!(stderr, "status: failed")?;
            writeln!(stderr, "code: {}", error.code())?;
            writeln!(stderr, "message: {error}")?;
        }
    }

    Ok(())
}

fn create_provider(command: &RunCommand) -> Result<CliTurnProvider, CliError> {
    match command.provider {
        ProviderKind::DeepSeek => Ok(CliTurnProvider::DeepSeek(Box::new(
            DeepSeekTurnProvider::new(command.max_output_tokens)?,
        ))),
        ProviderKind::Fixture => Ok(CliTurnProvider::Fixture(FixtureProvider::new(
            command.fixture,
        ))),
    }
}

#[derive(Debug)]
enum CliTurnProvider {
    DeepSeek(Box<DeepSeekTurnProvider>),
    Fixture(FixtureProvider),
}

impl TurnProvider for CliTurnProvider {
    fn complete(
        &mut self,
        request: TurnProviderRequest,
    ) -> Result<TurnProviderResponse, TurnProviderError> {
        match self {
            Self::DeepSeek(provider) => provider.complete(request),
            Self::Fixture(provider) => provider.complete(request),
        }
    }
}

#[derive(Debug)]
struct DeepSeekTurnProvider {
    adapter: DeepSeekApiAdapter,
    runtime: tokio::runtime::Runtime,
    max_output_tokens: u32,
}

impl DeepSeekTurnProvider {
    fn new(max_output_tokens: u32) -> Result<Self, CliError> {
        let config = DeepSeekApiConfig::from_env()?;
        let adapter = DeepSeekApiAdapter::new(config)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("deepseek-coder-cli-provider")
            .build()?;

        Ok(Self {
            adapter,
            runtime,
            max_output_tokens,
        })
    }
}

impl TurnProvider for DeepSeekTurnProvider {
    fn complete(
        &mut self,
        request: TurnProviderRequest,
    ) -> Result<TurnProviderResponse, TurnProviderError> {
        let mut chat_request = self
            .adapter
            .new_chat_request(request.messages)
            .map_err(|error| TurnProviderError::new(error.to_string()))?
            .with_max_tokens(self.max_output_tokens);
        chat_request = chat_request.with_tools(executable_chat_tools()?);

        let response = self
            .runtime
            .block_on(self.adapter.create_chat_completion(chat_request))
            .map_err(|error| TurnProviderError::new(error.to_string()))?;
        let choice =
            response.choices.into_iter().next().ok_or_else(|| {
                TurnProviderError::new("DeepSeek response did not include choices")
            })?;

        Ok(TurnProviderResponse::tool_calls(
            choice.message.content,
            choice.message.reasoning_content,
            choice.message.tool_calls.unwrap_or_default(),
        ))
    }
}

fn executable_chat_tools() -> Result<Vec<ChatTool>, TurnProviderError> {
    BUILTIN_TOOLS
        .iter()
        .filter(|tool| tool.implementation_status == ToolImplementationStatus::ExecutorImplemented)
        .map(|tool| {
            let parameters = serde_json::from_str(tool.argument_schema).map_err(|source| {
                TurnProviderError::new(format!(
                    "tool `{}` argument schema is invalid JSON: {source}",
                    tool.name.as_str()
                ))
            })?;
            Ok(ChatTool::function(ChatFunctionDefinition {
                name: tool.name.as_str().to_owned(),
                description: tool.description.to_owned(),
                parameters,
            }))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureKind {
    Final,
    Readme,
    Patch,
}

#[derive(Debug)]
struct FixtureProvider {
    responses: VecDeque<TurnProviderResponse>,
}

impl FixtureProvider {
    fn new(fixture: FixtureKind) -> Self {
        let responses = match fixture {
            FixtureKind::Final => vec![TurnProviderResponse::final_text(
                "Fixture provider completed without tool calls.",
            )],
            FixtureKind::Readme => vec![
                TurnProviderResponse::tool_calls(
                    None,
                    Some("Read README.md before answering.".to_owned()),
                    vec![ChatToolCall::function(
                        "call_readme",
                        "read_file",
                        r#"{"path":"README.md"}"#,
                    )],
                ),
                TurnProviderResponse::final_text(
                    "Fixture provider read README.md and completed the run.",
                ),
            ],
            FixtureKind::Patch => vec![
                TurnProviderResponse::tool_calls(
                    None,
                    Some("Apply the deterministic CLI smoke patch.".to_owned()),
                    vec![ChatToolCall::function(
                        "call_patch",
                        "apply_patch",
                        json!({
                            "unifiedDiff": concat!(
                                "--- a/CLI_SMOKE.txt\n",
                                "+++ b/CLI_SMOKE.txt\n",
                                "@@ -1 +1 @@\n",
                                "-old\n",
                                "+new\n",
                            ),
                            "expectedFiles": ["CLI_SMOKE.txt"],
                        })
                        .to_string(),
                    )],
                ),
                TurnProviderResponse::final_text(
                    "Fixture provider applied CLI_SMOKE.txt and completed the run.",
                ),
            ],
        };

        Self {
            responses: responses.into(),
        }
    }
}

impl TurnProvider for FixtureProvider {
    fn complete(
        &mut self,
        _request: TurnProviderRequest,
    ) -> Result<TurnProviderResponse, TurnProviderError> {
        self.responses
            .pop_front()
            .ok_or_else(|| TurnProviderError::new("fixture provider has no response"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    DeepSeek,
    Fixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VerificationOutcome {
    status: ToolStatus,
    exit_code: Option<i32>,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("I/O error: {0}")]
    StdIo(#[from] io::Error),
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("DeepSeek provider configuration failed: {0}")]
    DeepSeek(#[from] deepseek_coder_agent_core::provider::deepseek_api::DeepSeekApiError),
    #[error("run log failed: {0}")]
    RunLog(#[from] RunLogError),
    #[error("agent turn failed: {0}")]
    Turn(#[from] AgentTurnLoopError),
    #[error("tool execution failed: {0}")]
    ToolExecution(#[from] ToolExecutionError),
    #[error("RPC event bridge failed: {0}")]
    Rpc(#[from] AgentRpcError),
    #[error("verification command failed with exit code {exit_code:?}")]
    VerificationFailed { exit_code: Option<i32> },
    #[error("agent returned an empty final message")]
    EmptyFinalMessage,
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, CliError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("{option} requires a value")))
}

fn parse_mode(value: &str) -> Result<AgentRunMode, CliError> {
    match value {
        "plan" => Ok(AgentRunMode::Plan),
        "edit" => Ok(AgentRunMode::Edit),
        "review" => Ok(AgentRunMode::Review),
        "ask" => Ok(AgentRunMode::Ask),
        _ => Err(CliError::Usage(format!("unsupported mode `{value}`"))),
    }
}

fn parse_provider(value: &str) -> Result<ProviderKind, CliError> {
    match value {
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "fixture" => Ok(ProviderKind::Fixture),
        _ => Err(CliError::Usage(format!("unsupported provider `{value}`"))),
    }
}

fn parse_fixture(value: &str) -> Result<FixtureKind, CliError> {
    match value {
        "final" => Ok(FixtureKind::Final),
        "readme" => Ok(FixtureKind::Readme),
        "patch" => Ok(FixtureKind::Patch),
        _ => Err(CliError::Usage(format!("unsupported fixture `{value}`"))),
    }
}

fn parse_u64(value: &str) -> Result<u64, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("expected positive integer, got `{value}`")))
}

fn parse_u32(value: &str) -> Result<u32, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("expected positive integer, got `{value}`")))
}

fn parse_usize(value: &str) -> Result<usize, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("expected positive integer, got `{value}`")))
}

fn generate_id(prefix: &str) -> Result<String, CliError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| CliError::Io {
            path: PathBuf::from("system-clock"),
            source: io::Error::other(source),
        })?
        .as_millis();
    Ok(format!("{prefix}_{}_{}", std::process::id(), millis))
}

fn help_text() -> String {
    format!(
        "{name}\n\nUsage:\n  deepseek-coder run [options] <task>\n\n{}",
        run_help_text(),
        name = AGENT_METADATA.name
    )
}

fn run_help_text() -> String {
    [
        "Run options:",
        "  --workspace <path>          Workspace root. Defaults to current directory.",
        "  --provider <deepseek|fixture>",
        "  --fixture <final|readme|patch>",
        "  --mode <plan|edit|review|ask>",
        "  --run-id <id>",
        "  --turn-id <id>",
        "  --auto-approve, -y          Allow write/exec approvals without prompting.",
        "  --verify <command>          Run an explicit verification command after success.",
        "  --json                      Emit agent.event JSON-RPC notifications.",
        "  --max-input-tokens <n>",
        "  --max-model-turns <n>",
        "  --max-output-tokens <n>",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use deepseek_coder_agent_core::run_log::REDACTED_VALUE;
    use deepseek_coder_agent_core::run_log::RunLogStore;
    use serde_json::Value;

    use super::{CliCommand, ProviderKind, RunCommand, run_cli};

    #[test]
    fn parses_run_command_options() {
        let args = vec![
            "deepseek-coder".to_owned(),
            "run".to_owned(),
            "--provider".to_owned(),
            "fixture".to_owned(),
            "--fixture".to_owned(),
            "readme".to_owned(),
            "--mode".to_owned(),
            "ask".to_owned(),
            "--run-id".to_owned(),
            "run_test".to_owned(),
            "Read".to_owned(),
            "README".to_owned(),
        ];

        let command = CliCommand::parse(&args).expect("command should parse");

        match command {
            CliCommand::Run(RunCommand {
                task,
                provider,
                run_id,
                ..
            }) => {
                assert_eq!(task, "Read README");
                assert_eq!(provider, ProviderKind::Fixture);
                assert_eq!(run_id, "run_test");
            }
            CliCommand::Help => panic!("expected run command"),
        }
    }

    #[test]
    fn fixture_run_reads_readme_and_writes_run_log() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "hello from cli\n");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_cli(
            [
                "deepseek-coder",
                "run",
                "--provider",
                "fixture",
                "--fixture",
                "readme",
                "--workspace",
                workspace.path_str(),
                "--run-id",
                "run_cli_read",
                "--turn-id",
                "turn_cli_read",
                "Read README",
            ],
            &mut stdout,
            &mut stderr,
        )
        .expect("fixture run should succeed");

        let stdout = String::from_utf8(stdout).expect("stdout should be UTF-8");
        assert!(stdout.contains("status: completed"));
        assert!(stdout.contains("Fixture provider read README.md"));
        assert!(stderr.is_empty());

        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let events = store.load_run("run_cli_read").expect("events should load");
        assert!(events.iter().any(|event| event.event_type == "run.started"));
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.completed")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "run.completed")
        );
    }

    #[test]
    fn fixture_patch_run_can_verify_and_emit_json_events() {
        let workspace = TestWorkspace::new();
        workspace.write("CLI_SMOKE.txt", "old\n");
        let verification_secret = verification_secret();
        let verify_command = verification_command(&verification_secret);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_cli(
            [
                "deepseek-coder",
                "run",
                "--provider",
                "fixture",
                "--fixture",
                "patch",
                "--auto-approve",
                "--json",
                "--workspace",
                workspace.path_str(),
                "--run-id",
                "run_cli_patch",
                "--turn-id",
                "turn_cli_patch",
                "--verify",
                verify_command.as_str(),
                "Patch smoke file",
            ],
            &mut stdout,
            &mut stderr,
        )
        .expect("fixture patch run should succeed");

        assert_eq!(workspace.read("CLI_SMOKE.txt"), "new\n");
        assert!(stderr.is_empty());

        let output = String::from_utf8(stdout).expect("stdout should be UTF-8");
        assert!(!output.contains(&verification_secret));
        assert!(output.contains(REDACTED_VALUE));
        let lines = output.lines().collect::<Vec<_>>();
        assert!(!lines.is_empty());
        let notifications = lines
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).expect("line should be JSON"))
            .collect::<Vec<_>>();
        assert!(
            notifications
                .iter()
                .all(|value| value["method"] == "agent.event")
        );
        assert!(notifications.iter().any(|value| {
            value["params"]["type"] == "verification.completed"
                && value["params"]["payload"]["status"] == "passed"
                && value["params"]["payload"]["stdout"]
                    .as_str()
                    .is_some_and(|stdout| stdout.contains(REDACTED_VALUE))
        }));
    }

    const VERIFICATION_SECRET_PREFIX: &str = "sk";
    const VERIFICATION_SECRET_SUFFIX: &str = "not-a-real-verification-secret-123";

    fn verification_secret() -> String {
        format!("{VERIFICATION_SECRET_PREFIX}-{VERIFICATION_SECRET_SUFFIX}")
    }

    #[cfg(windows)]
    fn verification_command(secret: &str) -> String {
        format!(
            "Write-Output '{secret}'; if ((Get-Content CLI_SMOKE.txt -Raw).Trim() -ne 'new') {{ exit 1 }}"
        )
    }

    #[cfg(not(windows))]
    fn verification_command(secret: &str) -> String {
        format!("printf '%s\\n' '{secret}'; test \"$(cat CLI_SMOKE.txt)\" = new")
    }

    struct TestWorkspace {
        path: std::path::PathBuf,
        path_string: String,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let unique = format!(
                "deepseek-coder-cli-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock should be after epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp workspace should be created");
            let path_string = path.display().to_string();
            Self { path, path_string }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }

        fn path_str(&self) -> &str {
            &self.path_string
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent should be created");
            }
            fs::write(path, content).expect("file should be written");
        }

        fn read(&self, relative: &str) -> String {
            fs::read_to_string(self.path.join(relative)).expect("file should read")
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
