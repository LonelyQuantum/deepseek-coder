#![forbid(unsafe_code)]

use std::{
    collections::VecDeque,
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use deepseek_coder_agent_core::{
    AGENT_METADATA,
    cancellation::CancellationToken,
    context::ContextBuildError,
    provider::deepseek_api::{
        ChatCompletionStream, ChatFunctionDefinition, ChatTool, ChatToolCall,
        ChatToolCallAccumulator, DeepSeekApiAdapter, DeepSeekApiConfig, StreamEvent,
        ThinkingConfig,
    },
    reasoning::ReasoningContentMode,
    run_log::{RunLog, RunLogError, RunLogStore},
    tool::{BUILTIN_TOOLS, ToolImplementationStatus},
    tool_execution::{
        ShellArgs, ToolExecutionError, ToolStatus, WorkspaceToolExecutor,
        redacted_tool_result_value,
    },
    turn_loop::{
        AgentRunMode, AgentTurnInput, AgentTurnLoop, AgentTurnLoopConfig, AgentTurnLoopError,
        AgentTurnOutcome, ApprovalDecision, ApprovalPolicy, ApprovalPolicyError, AutoApprovePolicy,
        NoopTurnEventSink, TurnApprovalRequest, TurnEventSink, TurnEventSinkError, TurnProvider,
        TurnProviderDelta, TurnProviderError, TurnProviderEvent, TurnProviderFuture,
        TurnProviderRequest, TurnProviderResponse, TurnProviderStream,
        turn_provider_response_stream,
    },
};
use deepseek_coder_agent_rpc::{
    AgentRpcError, AgentRpcHandlerError, AgentTurnLoopRpcHandler, JSON_RPC_INTERNAL_ERROR,
    JSON_RPC_INVALID_PARAMS, JsonRpcErrorObject, JsonRpcErrorResponse, RPC_APPROVAL_DENIED,
    RPC_CONTEXT_BUDGET_EXCEEDED, RPC_INTERNAL_INVARIANT, RPC_INVALID_TOOL_ARGUMENTS,
    RPC_PROVIDER_ERROR, RPC_RUN_ALREADY_ACTIVE, RPC_RUN_CANCELED, RPC_RUN_NOT_FOUND,
    RPC_TOOL_EXECUTION_FAILED, RpcTurnProviderFactory, SendTurnParams, StdioEventBridge,
    run_stdio_request_loop,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use thiserror::Error;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_024;
const DEFAULT_VERIFY_TIMEOUT_MS: u64 = 120_000;
const CLI_RUN_JSON_RPC_ID: &str = "cli.run";

pub fn run_cli<I, S, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
    E: Write,
{
    let mut stdin = io::empty();
    run_cli_with_input(args, &mut stdin, stdout, stderr)
}

pub fn run_cli_with_input<I, S, R, W, E>(
    args: I,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    R: BufRead,
    W: Write,
    E: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let command = match CliCommand::parse(&args) {
        Ok(command) => command,
        Err(error) if args_request_json_run(&args) => {
            emit_json_rpc_error(stdout, &error, None)?;
            return Err(CliError::JsonRpcErrorReported {
                source: Box::new(error),
            });
        }
        Err(error) => return Err(error),
    };

    match command {
        CliCommand::Help => {
            writeln!(stdout, "{}", help_text())?;
            Ok(())
        }
        CliCommand::Run(command) => run_command(command, stdin, stdout, stderr),
        CliCommand::Rpc(command) => run_rpc_command(command, stdin, stdout),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Help,
    Run(RunCommand),
    Rpc(RpcCommand),
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
            "rpc" => Ok(Self::Rpc(RpcCommand::parse(args.cloned().collect())?)),
            other => Err(CliError::Usage(format!(
                "unknown command `{other}`\n\n{}",
                help_text()
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RpcCommand {
    provider: ProviderKind,
    fixture: FixtureKind,
    max_input_tokens: u64,
    max_model_turns: usize,
    max_output_tokens: u32,
    thinking: ThinkingKind,
}

impl RpcCommand {
    fn parse(args: Vec<String>) -> Result<Self, CliError> {
        let mut provider = ProviderKind::DeepSeek;
        let mut fixture = FixtureKind::Readme;
        let mut max_input_tokens = AgentTurnLoopConfig::default().max_input_tokens;
        let mut max_model_turns = AgentTurnLoopConfig::default().max_model_turns;
        let mut max_output_tokens = DEFAULT_MAX_OUTPUT_TOKENS;
        let mut thinking = ThinkingKind::Enabled;
        let mut index = 0;

        while index < args.len() {
            let arg = &args[index];
            match arg.as_str() {
                "-h" | "--help" => {
                    return Err(CliError::Usage(rpc_help_text()));
                }
                "--provider" => {
                    provider = parse_provider(&next_value(&args, &mut index, "--provider")?)?;
                }
                "--fixture" => {
                    fixture = parse_fixture(&next_value(&args, &mut index, "--fixture")?)?;
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
                "--thinking" => {
                    thinking = parse_thinking(&next_value(&args, &mut index, "--thinking")?)?;
                }
                value => {
                    return Err(CliError::Usage(format!("unknown rpc option `{value}`")));
                }
            }

            index += 1;
        }

        Ok(Self {
            provider,
            fixture,
            max_input_tokens,
            max_model_turns,
            max_output_tokens,
            thinking,
        })
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
    thinking: ThinkingKind,
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
        let mut thinking = ThinkingKind::Enabled;
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
                "--thinking" => {
                    thinking = parse_thinking(&next_value(&args, &mut index, "--thinking")?)?;
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
            thinking,
        })
    }
}

fn run_command<R, W, E>(
    command: RunCommand,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError>
where
    R: BufRead,
    W: Write,
    E: Write,
{
    let result = run_command_inner(&command, stdin, stdout, stderr);
    if !command.json_events {
        return result;
    }

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            emit_json_rpc_error(stdout, &error, Some((&command.run_id, &command.turn_id)))?;
            Err(CliError::JsonRpcErrorReported {
                source: Box::new(error),
            })
        }
    }
}

fn run_command_inner<R, W, E>(
    command: &RunCommand,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<(), CliError>
where
    R: BufRead,
    W: Write,
    E: Write,
{
    let workspace = fs::canonicalize(&command.workspace).map_err(|source| CliError::Io {
        path: command.workspace.clone(),
        source,
    })?;
    let store = RunLogStore::new(&workspace)?;
    let mut run_log = store.create_run(command.run_id.clone())?;
    let provider = create_provider(command)?;
    let config = AgentTurnLoopConfig {
        max_input_tokens: command.max_input_tokens,
        max_model_turns: command.max_model_turns,
        reasoning_mode: reasoning_mode_for_thinking(command.thinking),
    };
    let input =
        AgentTurnInput::new(command.turn_id.clone(), command.task.clone()).with_mode(command.mode);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_name("deepseek-coder-cli")
        .build()?;

    let (turn_result, verification_result) = if command.json_events {
        let mut event_sink = StdioEventBridge::new(stdout);
        let context = RunExecutionContext {
            workspace: &workspace,
            auto_approve: command.auto_approve,
            approval_input: stdin,
            approval_output: stderr,
            run_log: &mut run_log,
            event_sink: &mut event_sink,
        };
        let turn_result =
            runtime.block_on(run_with_selected_policy(context, provider, config, input));
        let verification_result = match (&turn_result, &command.verify_command) {
            (Ok(_), Some(verify_command)) => run_verification_with_event_sink(
                &workspace,
                &command.turn_id,
                verify_command,
                command.verify_timeout_ms,
                &mut run_log,
                &mut event_sink,
            )
            .map(Some),
            _ => Ok(None),
        };
        (turn_result, verification_result)
    } else {
        let mut event_sink = NoopTurnEventSink;
        let context = RunExecutionContext {
            workspace: &workspace,
            auto_approve: command.auto_approve,
            approval_input: stdin,
            approval_output: stderr,
            run_log: &mut run_log,
            event_sink: &mut event_sink,
        };
        let turn_result =
            runtime.block_on(run_with_selected_policy(context, provider, config, input));
        let verification_result = match (&turn_result, &command.verify_command) {
            (Ok(_), Some(verify_command)) => run_verification_with_event_sink(
                &workspace,
                &command.turn_id,
                verify_command,
                command.verify_timeout_ms,
                &mut run_log,
                &mut event_sink,
            )
            .map(Some),
            _ => Ok(None),
        };
        emit_human_summary(
            command,
            run_log.events_path(),
            &turn_result,
            verification_result.as_ref().ok().and_then(Option::as_ref),
            stdout,
            stderr,
        )?;
        (turn_result, verification_result)
    };

    let outcome = turn_result?;
    verification_result?;

    if outcome.final_message.trim().is_empty() {
        return Err(CliError::EmptyFinalMessage);
    }

    Ok(())
}

fn run_rpc_command<R, W>(command: RpcCommand, stdin: &mut R, stdout: &mut W) -> Result<(), CliError>
where
    R: BufRead,
    W: Write,
{
    let config = AgentTurnLoopConfig {
        max_input_tokens: command.max_input_tokens,
        max_model_turns: command.max_model_turns,
        reasoning_mode: reasoning_mode_for_thinking(command.thinking),
    };
    let handler = AgentTurnLoopRpcHandler::new(CliRpcProviderFactory {
        provider: command.provider,
        fixture: command.fixture,
        max_output_tokens: command.max_output_tokens,
        thinking: command.thinking,
    })
    .with_config(config);

    run_stdio_request_loop(stdin, stdout, handler)?;
    Ok(())
}

struct RunExecutionContext<'a, R, E, S>
where
    R: BufRead,
    E: Write,
    S: TurnEventSink + ?Sized,
{
    workspace: &'a Path,
    auto_approve: bool,
    approval_input: &'a mut R,
    approval_output: &'a mut E,
    run_log: &'a mut RunLog,
    event_sink: &'a mut S,
}

async fn run_with_selected_policy<R, E, S>(
    context: RunExecutionContext<'_, R, E, S>,
    provider: CliTurnProvider,
    config: AgentTurnLoopConfig,
    input: AgentTurnInput,
) -> Result<AgentTurnOutcome, AgentTurnLoopError>
where
    R: BufRead,
    E: Write,
    S: TurnEventSink + ?Sized,
{
    let RunExecutionContext {
        workspace,
        auto_approve,
        approval_input,
        approval_output,
        run_log,
        event_sink,
    } = context;

    if auto_approve {
        run_with_policy(
            workspace,
            provider,
            AutoApprovePolicy,
            config,
            input,
            run_log,
            event_sink,
        )
        .await
    } else {
        let approval_policy = PromptApprovalPolicy::new(approval_input, approval_output);
        run_with_policy(
            workspace,
            provider,
            approval_policy,
            config,
            input,
            run_log,
            event_sink,
        )
        .await
    }
}

async fn run_with_policy<A, S>(
    workspace: &Path,
    provider: CliTurnProvider,
    approval_policy: A,
    config: AgentTurnLoopConfig,
    input: AgentTurnInput,
    run_log: &mut RunLog,
    event_sink: &mut S,
) -> Result<AgentTurnOutcome, AgentTurnLoopError>
where
    A: ApprovalPolicy,
    S: TurnEventSink + ?Sized,
{
    let mut loop_runner =
        AgentTurnLoop::with_approval_policy(workspace, provider, approval_policy)?
            .with_config(config);
    loop_runner
        .run_turn_with_event_sink(input, run_log, event_sink)
        .await
}

fn run_verification_with_event_sink<S>(
    workspace: &Path,
    turn_id: &str,
    command: &str,
    timeout_ms: u64,
    run_log: &mut RunLog,
    event_sink: &mut S,
) -> Result<VerificationOutcome, CliError>
where
    S: TurnEventSink + ?Sized,
{
    let verification_id = "verification_1";
    append_cli_event(
        run_log,
        event_sink,
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

    append_cli_event(
        run_log,
        event_sink,
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

fn append_cli_event(
    run_log: &mut RunLog,
    event_sink: &mut (impl TurnEventSink + ?Sized),
    event_type: impl Into<String>,
    turn_id: Option<String>,
    payload: serde_json::Value,
) -> Result<(), CliError> {
    let event = run_log.append(event_type, turn_id, payload)?;
    event_sink.on_event(&event)?;
    Ok(())
}

struct PromptApprovalPolicy<'io, R, W> {
    input: &'io mut R,
    output: &'io mut W,
}

impl<'io, R, W> PromptApprovalPolicy<'io, R, W> {
    fn new(input: &'io mut R, output: &'io mut W) -> Self {
        Self { input, output }
    }
}

impl<R, W> ApprovalPolicy for PromptApprovalPolicy<'_, R, W>
where
    R: BufRead,
    W: Write,
{
    fn decide(
        &mut self,
        request: &TurnApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalPolicyError> {
        writeln!(self.output, "Approval required")?;
        writeln!(self.output, "  id: {}", request.approval_id)?;
        writeln!(self.output, "  tool: {}", request.tool_name)?;
        writeln!(self.output, "  risk: {}", request.risk.as_str())?;
        writeln!(self.output, "  title: {}", request.title)?;
        writeln!(self.output, "  detail: {}", request.detail)?;
        if let Some(command) = &request.command {
            writeln!(self.output, "  command: {command}")?;
        }
        if let Some(paths) = &request.paths {
            writeln!(self.output, "  paths: {}", paths.join(", "))?;
        }
        writeln!(self.output, "  persistable: {}", request.persistable)?;

        loop {
            write!(self.output, "Approve this tool call? [y/N] ")?;
            self.output.flush()?;

            let mut answer = String::new();
            if self.input.read_line(&mut answer)? == 0 {
                return Ok(ApprovalDecision::Rejected {
                    reason: "approval prompt input ended".to_owned(),
                });
            }

            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => return Ok(ApprovalDecision::Approved),
                "" | "n" | "no" => {
                    return Ok(ApprovalDecision::Rejected {
                        reason: "rejected by user".to_owned(),
                    });
                }
                _ => {
                    writeln!(self.output, "Please answer y or n.")?;
                }
            }
        }
    }
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

fn emit_json_rpc_error<W>(
    stdout: &mut W,
    error: &CliError,
    run_context: Option<(&str, &str)>,
) -> Result<(), CliError>
where
    W: Write,
{
    let mut data = serde_json::Map::new();
    data.insert(
        "symbolicCode".to_owned(),
        Value::String(cli_error_symbolic_code(error).to_owned()),
    );
    data.insert(
        "kind".to_owned(),
        Value::String(cli_error_kind(error).to_owned()),
    );
    if let Some((run_id, turn_id)) = run_context {
        data.insert("runId".to_owned(), Value::String(run_id.to_owned()));
        data.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
    }
    if let CliError::VerificationFailed { exit_code } = error {
        data.insert(
            "exitCode".to_owned(),
            exit_code.map_or(Value::Null, |exit_code| json!(exit_code)),
        );
    }

    let error = JsonRpcErrorObject::new(cli_error_json_rpc_code(error), error.to_string())
        .with_data(Value::Object(data));
    let response = JsonRpcErrorResponse::new(Value::String(CLI_RUN_JSON_RPC_ID.to_owned()), error);
    let response = serde_json::to_string(&response)?;
    stdout.write_all(response.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn cli_error_json_rpc_code(error: &CliError) -> i64 {
    match error {
        CliError::Usage(_) => JSON_RPC_INVALID_PARAMS,
        CliError::StdIo(_) | CliError::Io { .. } | CliError::JsonSerialization(_) => {
            JSON_RPC_INTERNAL_ERROR
        }
        CliError::DeepSeek(_) | CliError::EmptyFinalMessage => RPC_PROVIDER_ERROR,
        CliError::RunLog(error) => run_log_error_json_rpc_code(error),
        CliError::Turn(error) => turn_loop_error_json_rpc_code(error),
        CliError::ToolExecution(_) | CliError::VerificationFailed { .. } => {
            RPC_TOOL_EXECUTION_FAILED
        }
        CliError::Rpc(_) | CliError::EventSink(_) => RPC_INTERNAL_INVARIANT,
        CliError::JsonRpcErrorReported { source } => cli_error_json_rpc_code(source),
    }
}

fn turn_loop_error_json_rpc_code(error: &AgentTurnLoopError) -> i64 {
    match error {
        AgentTurnLoopError::InvalidConfig { .. } => JSON_RPC_INVALID_PARAMS,
        AgentTurnLoopError::ContextBuild(ContextBuildError::RequiredContextExceedsBudget {
            ..
        }) => RPC_CONTEXT_BUDGET_EXCEEDED,
        AgentTurnLoopError::ContextBuild(
            ContextBuildError::InvalidMaxInputTokens
            | ContextBuildError::EmptyReason
            | ContextBuildError::InvalidPath { .. }
            | ContextBuildError::InvalidCommandId { .. }
            | ContextBuildError::DuplicateSingletonItem { .. }
            | ContextBuildError::DuplicateFilePath { .. }
            | ContextBuildError::DuplicateCommandId { .. },
        ) => JSON_RPC_INVALID_PARAMS,
        AgentTurnLoopError::ContextBuild(ContextBuildError::TokenCountOverflow) => {
            RPC_INTERNAL_INVARIANT
        }
        AgentTurnLoopError::Reasoning(_)
        | AgentTurnLoopError::Provider(_)
        | AgentTurnLoopError::ProviderStreamEndedWithoutCompletion
        | AgentTurnLoopError::ProviderCompletedMultipleTimes
        | AgentTurnLoopError::ProviderEventAfterCompletion
        | AgentTurnLoopError::MissingAssistantReasoningContent
        | AgentTurnLoopError::MaxModelTurnsExceeded { .. } => RPC_PROVIDER_ERROR,
        AgentTurnLoopError::RunLog(error) => run_log_error_json_rpc_code(error),
        AgentTurnLoopError::EventSink(_) | AgentTurnLoopError::ApprovalPolicy(_) => {
            RPC_INTERNAL_INVARIANT
        }
        AgentTurnLoopError::ToolExecution(_)
        | AgentTurnLoopError::UnknownTool { .. }
        | AgentTurnLoopError::UnsupportedTool { .. }
        | AgentTurnLoopError::Serialization(_) => RPC_TOOL_EXECUTION_FAILED,
        AgentTurnLoopError::InvalidToolArguments { .. } => RPC_INVALID_TOOL_ARGUMENTS,
        AgentTurnLoopError::Canceled { .. }
        | AgentTurnLoopError::ApprovalCanceled { .. }
        | AgentTurnLoopError::ApprovalExpired { .. } => RPC_RUN_CANCELED,
        AgentTurnLoopError::ApprovalRejected { .. } => RPC_APPROVAL_DENIED,
    }
}

fn run_log_error_json_rpc_code(error: &RunLogError) -> i64 {
    match error {
        RunLogError::RunAlreadyExists { .. } => RPC_RUN_ALREADY_ACTIVE,
        RunLogError::RunNotFound { .. } | RunLogError::RunSummaryNotFound { .. } => {
            RPC_RUN_NOT_FOUND
        }
        RunLogError::InvalidIdentifier { .. }
        | RunLogError::InvalidEventType { .. }
        | RunLogError::InvalidStatePath { .. }
        | RunLogError::WorkspaceRootNotDirectory { .. } => JSON_RPC_INVALID_PARAMS,
        _ => RPC_INTERNAL_INVARIANT,
    }
}

fn cli_error_symbolic_code(error: &CliError) -> &'static str {
    match error {
        CliError::Usage(_) => "E_INVALID_PARAMS",
        CliError::StdIo(_) | CliError::Io { .. } => "E_IO",
        CliError::DeepSeek(_) | CliError::EmptyFinalMessage => "E_PROVIDER_ERROR",
        CliError::RunLog(error) => run_log_error_symbolic_code(error),
        CliError::Turn(error) => turn_loop_error_symbolic_code(error),
        CliError::ToolExecution(_) => "E_TOOL_EXECUTION_FAILED",
        CliError::Rpc(_) => "E_RPC_ERROR",
        CliError::EventSink(_) => "E_EVENT_SINK",
        CliError::VerificationFailed { .. } => "E_VERIFICATION_FAILED",
        CliError::JsonSerialization(_) => "E_SERIALIZATION",
        CliError::JsonRpcErrorReported { source } => cli_error_symbolic_code(source),
    }
}

fn turn_loop_error_symbolic_code(error: &AgentTurnLoopError) -> &'static str {
    match error {
        AgentTurnLoopError::ContextBuild(ContextBuildError::RequiredContextExceedsBudget {
            ..
        }) => "E_CONTEXT_BUDGET_EXCEEDED",
        _ => error.code(),
    }
}

fn run_log_error_symbolic_code(error: &RunLogError) -> &'static str {
    match error {
        RunLogError::RunAlreadyExists { .. } => "E_RUN_ALREADY_ACTIVE",
        RunLogError::RunNotFound { .. } | RunLogError::RunSummaryNotFound { .. } => {
            "E_RUN_NOT_FOUND"
        }
        RunLogError::InvalidIdentifier { .. }
        | RunLogError::InvalidEventType { .. }
        | RunLogError::InvalidStatePath { .. }
        | RunLogError::WorkspaceRootNotDirectory { .. } => "E_INVALID_PARAMS",
        _ => "E_INTERNAL_INVARIANT",
    }
}

fn cli_error_kind(error: &CliError) -> &'static str {
    match error {
        CliError::Usage(_) => "usage",
        CliError::StdIo(_) | CliError::Io { .. } => "io",
        CliError::DeepSeek(_) | CliError::EmptyFinalMessage => "provider",
        CliError::RunLog(_) => "run_log",
        CliError::Turn(_) => "turn",
        CliError::ToolExecution(_) => "tool",
        CliError::Rpc(_) => "rpc",
        CliError::EventSink(_) => "event_sink",
        CliError::VerificationFailed { .. } => "verification",
        CliError::JsonSerialization(_) => "serialization",
        CliError::JsonRpcErrorReported { source } => cli_error_kind(source),
    }
}

fn args_request_json_run(args: &[String]) -> bool {
    if args.get(1).map(String::as_str) != Some("run") {
        return false;
    }

    for arg in args.iter().skip(2) {
        if arg == "--" {
            return false;
        }
        if arg == "--json" {
            return true;
        }
    }

    false
}

fn create_provider(command: &RunCommand) -> Result<CliTurnProvider, CliError> {
    create_cli_provider(
        command.provider,
        command.fixture,
        command.max_output_tokens,
        command.thinking,
    )
}

fn reasoning_mode_for_thinking(thinking: ThinkingKind) -> ReasoningContentMode {
    match thinking {
        ThinkingKind::Enabled => ReasoningContentMode::ThinkingEnabled,
        ThinkingKind::Disabled => ReasoningContentMode::ThinkingDisabled,
    }
}

fn create_cli_provider(
    provider: ProviderKind,
    fixture: FixtureKind,
    max_output_tokens: u32,
    thinking: ThinkingKind,
) -> Result<CliTurnProvider, CliError> {
    match provider {
        ProviderKind::DeepSeek => Ok(CliTurnProvider::DeepSeek(Box::new(
            DeepSeekTurnProvider::new(max_output_tokens, thinking)?,
        ))),
        ProviderKind::Fixture => Ok(CliTurnProvider::Fixture(FixtureProvider::new(fixture))),
    }
}

#[derive(Debug, Clone, Copy)]
struct CliRpcProviderFactory {
    provider: ProviderKind,
    fixture: FixtureKind,
    max_output_tokens: u32,
    thinking: ThinkingKind,
}

impl RpcTurnProviderFactory for CliRpcProviderFactory {
    type Provider = CliTurnProvider;

    fn create_provider(
        &mut self,
        _params: &SendTurnParams,
    ) -> Result<Self::Provider, AgentRpcHandlerError> {
        create_cli_provider(
            self.provider,
            self.fixture,
            self.max_output_tokens,
            self.thinking,
        )
        .map_err(|error| AgentRpcHandlerError::new(RPC_INTERNAL_INVARIANT, error.to_string()))
    }
}

#[derive(Debug)]
enum CliTurnProvider {
    DeepSeek(Box<DeepSeekTurnProvider>),
    Fixture(FixtureProvider),
}

impl TurnProvider for CliTurnProvider {
    fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_> {
        Box::pin(async move {
            match self {
                Self::DeepSeek(provider) => provider.complete_stream(request).await,
                Self::Fixture(provider) => provider.complete_stream(request).await,
            }
        })
    }
}

#[derive(Debug)]
struct DeepSeekTurnProvider {
    adapter: DeepSeekApiAdapter,
    max_output_tokens: u32,
    thinking: ThinkingKind,
}

impl DeepSeekTurnProvider {
    fn new(max_output_tokens: u32, thinking: ThinkingKind) -> Result<Self, CliError> {
        let config = DeepSeekApiConfig::from_env()?;
        let adapter = DeepSeekApiAdapter::new(config)?;

        Ok(Self {
            adapter,
            max_output_tokens,
            thinking,
        })
    }
}

impl TurnProvider for DeepSeekTurnProvider {
    fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_> {
        Box::pin(async move {
            let mut chat_request = self
                .adapter
                .new_chat_request(request.messages)
                .map_err(|error| TurnProviderError::new(error.to_string()))?
                .with_max_tokens(self.max_output_tokens);
            chat_request = match self.thinking {
                ThinkingKind::Enabled => chat_request.with_thinking(ThinkingConfig::enabled()),
                ThinkingKind::Disabled => chat_request.with_thinking(ThinkingConfig::disabled()),
            };
            chat_request = chat_request.with_tools(executable_chat_tools()?);

            let stream = self
                .adapter
                .create_chat_completion_stream(chat_request)
                .await
                .map_err(|error| TurnProviderError::new(error.to_string()))?;

            Ok(deepseek_chat_stream_to_turn_provider_stream(
                stream,
                request.cancellation_token,
            ))
        })
    }
}

fn deepseek_chat_stream_to_turn_provider_stream(
    mut stream: ChatCompletionStream,
    cancellation_token: CancellationToken,
) -> TurnProviderStream {
    Box::pin(async_stream::try_stream! {
        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut tool_call_accumulator = ChatToolCallAccumulator::new();

        while let Some(event) = stream.next().await {
            if cancellation_token.is_canceled() {
                Err(TurnProviderError::new(cancellation_token.cancellation_reason()))?;
            }
            match event.map_err(|error| TurnProviderError::new(error.to_string()))? {
                StreamEvent::Chunk(chunk) => {
                    for choice in chunk.choices {
                        let delta = choice.delta;
                        let mut provider_delta = TurnProviderDelta::new(None, None);

                        if let Some(delta_content) = delta.content
                            && !delta_content.is_empty()
                        {
                            content.push_str(&delta_content);
                            provider_delta.content = Some(delta_content);
                        }

                        if let Some(delta_reasoning_content) = delta.reasoning_content
                            && !delta_reasoning_content.is_empty()
                        {
                            reasoning_content.push_str(&delta_reasoning_content);
                            provider_delta.reasoning_content = Some(delta_reasoning_content);
                        }

                        if !provider_delta.is_empty() {
                            yield TurnProviderEvent::AssistantDelta(provider_delta);
                        }

                        if let Some(delta_tool_calls) = delta.tool_calls {
                            for tool_call_delta in delta_tool_calls {
                                tool_call_accumulator
                                    .append_delta(tool_call_delta)
                                    .map_err(|error| TurnProviderError::new(error.to_string()))?;
                            }
                        }
                    }
                }
                StreamEvent::Done => break,
            }
            if cancellation_token.is_canceled() {
                Err(TurnProviderError::new(cancellation_token.cancellation_reason()))?;
            }
        }
        if cancellation_token.is_canceled() {
            Err(TurnProviderError::new(cancellation_token.cancellation_reason()))?;
        }

        let tool_calls = tool_call_accumulator
            .finish()
            .map_err(|error| TurnProviderError::new(error.to_string()))?;

        yield TurnProviderEvent::Completed(TurnProviderResponse::tool_calls(
            non_empty_string(content),
            non_empty_string(reasoning_content),
            tool_calls,
        ));
    })
}

fn non_empty_string(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
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
    fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_> {
        Box::pin(async move {
            if request.cancellation_token.is_canceled() {
                return Err(TurnProviderError::new(
                    request.cancellation_token.cancellation_reason(),
                ));
            }
            let response = self
                .responses
                .pop_front()
                .ok_or_else(|| TurnProviderError::new("fixture provider has no response"))?;
            Ok(turn_provider_response_stream(response))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    DeepSeek,
    Fixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkingKind {
    Enabled,
    Disabled,
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
    #[error("event sink failed: {0}")]
    EventSink(#[from] TurnEventSinkError),
    #[error("verification command failed with exit code {exit_code:?}")]
    VerificationFailed { exit_code: Option<i32> },
    #[error("JSON serialization failed: {0}")]
    JsonSerialization(#[from] serde_json::Error),
    #[error("{source}")]
    JsonRpcErrorReported { source: Box<CliError> },
    #[error("agent returned an empty final message")]
    EmptyFinalMessage,
}

impl CliError {
    pub fn is_reported(&self) -> bool {
        matches!(self, Self::JsonRpcErrorReported { .. })
    }
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

fn parse_thinking(value: &str) -> Result<ThinkingKind, CliError> {
    match value {
        "enabled" => Ok(ThinkingKind::Enabled),
        "disabled" => Ok(ThinkingKind::Disabled),
        _ => Err(CliError::Usage(format!("unsupported thinking `{value}`"))),
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
        "{name}\n\nUsage:\n  deepseek-coder run [options] <task>\n  deepseek-coder rpc [options]\n\n{}\n\n{}",
        run_help_text(),
        rpc_help_text(),
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
        "  --thinking <enabled|disabled>",
    ]
    .join("\n")
}

fn rpc_help_text() -> String {
    [
        "RPC options:",
        "  --provider <deepseek|fixture>",
        "  --fixture <final|readme|patch>",
        "  --max-input-tokens <n>",
        "  --max-model-turns <n>",
        "  --max-output-tokens <n>",
        "  --thinking <enabled|disabled>",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use deepseek_coder_agent_core::{
        cancellation::CancellationToken,
        provider::deepseek_api::{
            ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta,
            ChatFunctionCallDelta, ChatToolCallDelta, ChatToolType, StreamEvent,
        },
        run_log::{REDACTED_VALUE, RunLogStore},
        test_helpers::TestWorkspace,
        turn_loop::TurnProviderEvent,
    };
    use deepseek_coder_agent_rpc::{
        PROTOCOL_VERSION, RPC_APPROVAL_DENIED, RPC_TOOL_EXECUTION_FAILED,
    };
    use futures_util::{StreamExt, stream};
    use serde_json::{Value, json};

    use super::{
        CliCommand, ProviderKind, RunCommand, deepseek_chat_stream_to_turn_provider_stream,
        run_cli, run_cli_with_input,
    };

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
            "--thinking".to_owned(),
            "disabled".to_owned(),
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
                thinking,
                ..
            }) => {
                assert_eq!(task, "Read README");
                assert_eq!(provider, ProviderKind::Fixture);
                assert_eq!(run_id, "run_test");
                assert_eq!(thinking, super::ThinkingKind::Disabled);
            }
            CliCommand::Help | CliCommand::Rpc(_) => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_rpc_command_options() {
        let args = vec![
            "deepseek-coder".to_owned(),
            "rpc".to_owned(),
            "--provider".to_owned(),
            "fixture".to_owned(),
            "--fixture".to_owned(),
            "final".to_owned(),
            "--thinking".to_owned(),
            "disabled".to_owned(),
            "--max-model-turns".to_owned(),
            "2".to_owned(),
        ];

        let command = CliCommand::parse(&args).expect("command should parse");

        match command {
            CliCommand::Rpc(command) => {
                assert_eq!(command.provider, ProviderKind::Fixture);
                assert_eq!(command.fixture, super::FixtureKind::Final);
                assert_eq!(command.thinking, super::ThinkingKind::Disabled);
                assert_eq!(command.max_model_turns, 2);
            }
            _ => panic!("expected rpc command"),
        }
    }

    #[test]
    fn fixture_run_reads_readme_and_writes_run_log() {
        let workspace = TestWorkspace::new("cli");
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
        let workspace = TestWorkspace::new("cli");
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

        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let events = store.load_run("run_cli_patch").expect("events should load");
        assert_eq!(notifications.len(), events.len());
        let notification_seqs = notifications
            .iter()
            .map(|value| value["params"]["seq"].as_u64())
            .collect::<Vec<_>>();
        let persisted_seqs = events
            .iter()
            .map(|event| Some(event.seq))
            .collect::<Vec<_>>();
        assert_eq!(notification_seqs, persisted_seqs);
    }

    #[test]
    fn fixture_patch_run_json_rejection_emits_json_rpc_error() {
        let workspace = TestWorkspace::new("cli");
        workspace.write("CLI_SMOKE.txt", "old\n");
        let mut stdin = std::io::Cursor::new(b"n\n".to_vec());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = run_cli_with_input(
            [
                "deepseek-coder",
                "run",
                "--provider",
                "fixture",
                "--fixture",
                "patch",
                "--json",
                "--workspace",
                workspace.path_str(),
                "--run-id",
                "run_cli_json_reject",
                "--turn-id",
                "turn_cli_json_reject",
                "Patch smoke file",
            ],
            &mut stdin,
            &mut stdout,
            &mut stderr,
        )
        .expect_err("rejected json run should fail");

        assert!(error.is_reported());
        assert_eq!(workspace.read("CLI_SMOKE.txt"), "old\n");
        let stderr = String::from_utf8(stderr).expect("stderr should be UTF-8");
        assert!(stderr.contains("Approval required"));
        assert!(!stderr.contains("status: failed"));

        let lines = json_lines(stdout);
        assert!(lines.len() >= 2);
        let (events, error_response) = lines.split_at(lines.len() - 1);
        assert!(events.iter().all(|value| value["method"] == "agent.event"));
        assert!(events.iter().any(|value| {
            value["params"]["type"] == "run.failed"
                && value["params"]["payload"]["code"] == "E_APPROVAL_REJECTED"
        }));
        let error_response = &error_response[0];
        assert_eq!(error_response["jsonrpc"], "2.0");
        assert_eq!(error_response["id"], "cli.run");
        assert_eq!(error_response["error"]["code"], RPC_APPROVAL_DENIED);
        assert!(
            error_response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("rejected by user"))
        );
        assert_eq!(
            error_response["error"]["data"]["symbolicCode"],
            "E_APPROVAL_REJECTED"
        );
        assert_eq!(error_response["error"]["data"]["kind"], "turn");
        assert_eq!(
            error_response["error"]["data"]["runId"],
            "run_cli_json_reject"
        );
        assert_eq!(
            error_response["error"]["data"]["turnId"],
            "turn_cli_json_reject"
        );
    }

    #[test]
    fn fixture_patch_run_json_verification_failure_emits_json_rpc_error() {
        let workspace = TestWorkspace::new("cli");
        workspace.write("CLI_SMOKE.txt", "old\n");
        let verify_command = verification_failure_command();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = run_cli(
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
                "run_cli_json_verify_failed",
                "--turn-id",
                "turn_cli_json_verify_failed",
                "--verify",
                verify_command.as_str(),
                "Patch smoke file",
            ],
            &mut stdout,
            &mut stderr,
        )
        .expect_err("verification failure should fail");

        assert!(error.is_reported());
        assert_eq!(workspace.read("CLI_SMOKE.txt"), "new\n");
        assert!(stderr.is_empty());

        let lines = json_lines(stdout);
        assert!(lines.len() >= 2);
        let (events, error_response) = lines.split_at(lines.len() - 1);
        assert!(events.iter().all(|value| value["method"] == "agent.event"));
        assert!(events.iter().any(|value| {
            value["params"]["type"] == "verification.completed"
                && value["params"]["payload"]["status"] == "failed"
        }));
        let error_response = &error_response[0];
        assert_eq!(error_response["jsonrpc"], "2.0");
        assert_eq!(error_response["id"], "cli.run");
        assert_eq!(error_response["error"]["code"], RPC_TOOL_EXECUTION_FAILED);
        assert_eq!(
            error_response["error"]["data"]["symbolicCode"],
            "E_VERIFICATION_FAILED"
        );
        assert_eq!(error_response["error"]["data"]["kind"], "verification");
        assert_eq!(error_response["error"]["data"]["exitCode"], 7);
        assert_eq!(
            error_response["error"]["data"]["runId"],
            "run_cli_json_verify_failed"
        );
    }

    #[test]
    fn fixture_patch_run_prompts_for_approval_and_applies_when_approved() {
        let workspace = TestWorkspace::new("cli");
        workspace.write("CLI_SMOKE.txt", "old\n");
        let mut stdin = std::io::Cursor::new(b"y\n".to_vec());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_cli_with_input(
            [
                "deepseek-coder",
                "run",
                "--provider",
                "fixture",
                "--fixture",
                "patch",
                "--workspace",
                workspace.path_str(),
                "--run-id",
                "run_cli_prompt_approve",
                "--turn-id",
                "turn_cli_prompt",
                "Patch smoke file",
            ],
            &mut stdin,
            &mut stdout,
            &mut stderr,
        )
        .expect("approved prompt run should succeed");

        assert_eq!(workspace.read("CLI_SMOKE.txt"), "new\n");
        let stderr = String::from_utf8(stderr).expect("stderr should be UTF-8");
        assert!(stderr.contains("Approval required"));
        assert!(stderr.contains("Approve this tool call?"));
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let events = store
            .load_run("run_cli_prompt_approve")
            .expect("events should load");
        assert!(events.iter().any(|event| {
            event.event_type == "tool.approvalResolved" && event.payload["decision"] == "approved"
        }));
    }

    #[test]
    fn fixture_patch_run_prompts_for_approval_and_rejects() {
        let workspace = TestWorkspace::new("cli");
        workspace.write("CLI_SMOKE.txt", "old\n");
        let mut stdin = std::io::Cursor::new(b"n\n".to_vec());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error = run_cli_with_input(
            [
                "deepseek-coder",
                "run",
                "--provider",
                "fixture",
                "--fixture",
                "patch",
                "--workspace",
                workspace.path_str(),
                "--run-id",
                "run_cli_prompt_reject",
                "--turn-id",
                "turn_cli_prompt",
                "Patch smoke file",
            ],
            &mut stdin,
            &mut stdout,
            &mut stderr,
        )
        .expect_err("rejected prompt run should fail");

        assert!(error.to_string().contains("rejected by user"));
        assert_eq!(workspace.read("CLI_SMOKE.txt"), "old\n");
        let stderr = String::from_utf8(stderr).expect("stderr should be UTF-8");
        assert!(stderr.contains("status: failed"));
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let events = store
            .load_run("run_cli_prompt_reject")
            .expect("events should load");
        assert!(events.iter().any(|event| {
            event.event_type == "tool.approvalResolved"
                && event.payload["decision"] == "rejected"
                && event.payload["reason"] == "rejected by user"
        }));
        assert!(events.iter().any(|event| event.event_type == "run.failed"));
    }

    #[test]
    fn rpc_command_runs_fixture_turn_loop_handler() {
        let workspace = TestWorkspace::new("cli");
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": "init_1",
                "method": "agent.initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "client": {
                        "name": "cli-test",
                        "version": "0.1.0",
                        "frontend": "cli"
                    },
                    "workspaceRoot": workspace.path_str(),
                    "workspaceTrusted": true
                }
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "turn_1",
                "method": "agent.sendTurn",
                "params": {
                    "runId": "run_cli_rpc",
                    "message": "Say hello",
                    "mode": "ask"
                }
            })
            .to_string(),
        ]
        .join("\n");
        let mut stdin = std::io::Cursor::new(input.into_bytes());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_cli_with_input(
            [
                "deepseek-coder",
                "rpc",
                "--provider",
                "fixture",
                "--fixture",
                "final",
            ],
            &mut stdin,
            &mut stdout,
            &mut stderr,
        )
        .expect("fixture rpc command should succeed");

        assert!(stderr.is_empty());
        let lines = String::from_utf8(stdout)
            .expect("stdout should be UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("line should be JSON"))
            .collect::<Vec<_>>();
        assert_eq!(lines[0]["id"], "init_1");
        assert_eq!(lines[1]["id"], "turn_1");
        assert_eq!(lines[1]["result"]["accepted"], true);
        assert!(lines.iter().any(|line| {
            line["method"] == "agent.event"
                && line["params"]["type"] == "run.completed"
                && line["params"]["payload"]["summary"]
                    == "Fixture provider completed without tool calls."
        }));

        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let events = store.load_run("run_cli_rpc").expect("events should load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "run.completed")
        );
    }

    #[test]
    fn deepseek_stream_wrapper_aggregates_deltas() {
        let stream = Box::pin(stream::iter([
            Ok(StreamEvent::Chunk(chat_chunk(Some("hel"), Some("think")))),
            Ok(StreamEvent::Chunk(chat_chunk(Some("lo"), None))),
            Ok(StreamEvent::Done),
        ]));
        let mut stream =
            deepseek_chat_stream_to_turn_provider_stream(stream, CancellationToken::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime should build");

        let events = runtime.block_on(async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("provider stream event should be ok"));
            }
            events
        });

        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            TurnProviderEvent::AssistantDelta(delta)
                if delta.content.as_deref() == Some("hel")
                    && delta.reasoning_content.as_deref() == Some("think")
        ));
        assert!(matches!(
            &events[1],
            TurnProviderEvent::AssistantDelta(delta)
                if delta.content.as_deref() == Some("lo")
                    && delta.reasoning_content.is_none()
        ));
        assert!(matches!(
            &events[2],
            TurnProviderEvent::Completed(response)
                if response.content.as_deref() == Some("hello")
                    && response.reasoning_content.as_deref() == Some("think")
        ));
    }

    #[test]
    fn deepseek_stream_wrapper_accumulates_tool_call_deltas() {
        let stream = Box::pin(stream::iter([
            Ok(StreamEvent::Chunk(chat_tool_call_chunk(vec![
                tool_call_delta(
                    0,
                    Some("call_read"),
                    Some(ChatToolType::Function),
                    Some("read_file"),
                    Some("{\"path\""),
                ),
            ]))),
            Ok(StreamEvent::Chunk(chat_tool_call_chunk(vec![
                tool_call_delta(0, None, None, None, Some(":\"README.md\"}")),
            ]))),
            Ok(StreamEvent::Done),
        ]));
        let mut stream =
            deepseek_chat_stream_to_turn_provider_stream(stream, CancellationToken::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime should build");

        let events = runtime.block_on(async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("provider stream event should be ok"));
            }
            events
        });

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            TurnProviderEvent::Completed(response)
                if response.tool_calls.len() == 1
                    && response.tool_calls[0].id == "call_read"
                    && response.tool_calls[0].function.name == "read_file"
                    && response.tool_calls[0].function.arguments == r#"{"path":"README.md"}"#
        ));
    }

    #[test]
    fn deepseek_stream_wrapper_rejects_incomplete_tool_call_delta() {
        let stream = Box::pin(stream::iter([
            Ok(StreamEvent::Chunk(chat_tool_call_chunk(vec![
                tool_call_delta(
                    0,
                    None,
                    Some(ChatToolType::Function),
                    Some("read_file"),
                    Some("{}"),
                ),
            ]))),
            Ok(StreamEvent::Done),
        ]));
        let mut stream =
            deepseek_chat_stream_to_turn_provider_stream(stream, CancellationToken::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime should build");

        let error = runtime
            .block_on(async { stream.next().await.expect("stream should yield an error") })
            .expect_err("incomplete tool call must fail");

        assert!(
            error
                .to_string()
                .contains("tool call stream ended before index 0 emitted an id")
        );
    }

    fn chat_chunk(content: Option<&str>, reasoning_content: Option<&str>) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chunk_test".to_owned(),
            object: "chat.completion.chunk".to_owned(),
            created: 1,
            model: "deepseek-v4-pro".to_owned(),
            system_fingerprint: None,
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatCompletionDelta {
                    role: None,
                    content: content.map(str::to_owned),
                    reasoning_content: reasoning_content.map(str::to_owned),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    fn chat_tool_call_chunk(tool_calls: Vec<ChatToolCallDelta>) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: "chunk_test".to_owned(),
            object: "chat.completion.chunk".to_owned(),
            created: 1,
            model: "deepseek-v4-pro".to_owned(),
            system_fingerprint: None,
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatCompletionDelta {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(tool_calls),
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    fn tool_call_delta(
        index: u32,
        id: Option<&str>,
        kind: Option<ChatToolType>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> ChatToolCallDelta {
        ChatToolCallDelta {
            index,
            id: id.map(str::to_owned),
            kind,
            function: (name.is_some() || arguments.is_some()).then(|| ChatFunctionCallDelta {
                name: name.map(str::to_owned),
                arguments: arguments.map(str::to_owned),
            }),
        }
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

    #[cfg(windows)]
    fn verification_failure_command() -> String {
        "exit 7".to_owned()
    }

    #[cfg(not(windows))]
    fn verification_failure_command() -> String {
        "exit 7".to_owned()
    }

    fn json_lines(output: Vec<u8>) -> Vec<Value> {
        String::from_utf8(output)
            .expect("stdout should be UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("line should be JSON"))
            .collect()
    }
}
