use std::path::Path;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::{
    approval::{ApprovalRequirement, RiskLevel},
    context::{ContextBuildError, ContextBuilder, ContextBuilderConfig, ContextItem},
    provider::deepseek_api::{ChatMessage, ChatToolCall},
    reasoning::{
        ReasoningContentError, ReasoningContentMode, ReasoningContentState,
        ReasoningContentStateMachine,
    },
    run_log::{RunLog, RunLogError},
    tool::{ToolDefinition, find_builtin_tool},
    tool_execution::{
        ApplyPatchArgs, ShellArgs, ToolExecutionError, ToolStatus, WorkspaceToolExecutor,
        redacted_tool_result_value,
    },
};

#[derive(Debug)]
pub struct AgentTurnLoop<P, A> {
    provider: P,
    approval_policy: A,
    tools: WorkspaceToolExecutor,
    reasoning: ReasoningContentStateMachine,
    config: AgentTurnLoopConfig,
}

impl<P> AgentTurnLoop<P, RejectAllApprovalPolicy> {
    pub fn new(workspace_root: impl AsRef<Path>, provider: P) -> Result<Self, AgentTurnLoopError> {
        Self::with_approval_policy(workspace_root, provider, RejectAllApprovalPolicy)
    }
}

impl<P, A> AgentTurnLoop<P, A> {
    pub fn with_approval_policy(
        workspace_root: impl AsRef<Path>,
        provider: P,
        approval_policy: A,
    ) -> Result<Self, AgentTurnLoopError> {
        Ok(Self {
            provider,
            approval_policy,
            tools: WorkspaceToolExecutor::new(workspace_root)?,
            reasoning: ReasoningContentStateMachine::default(),
            config: AgentTurnLoopConfig::default(),
        })
    }

    pub fn with_config(mut self, config: AgentTurnLoopConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_reasoning(mut self, reasoning: ReasoningContentStateMachine) -> Self {
        self.reasoning = reasoning;
        self
    }
}

impl<P, A> AgentTurnLoop<P, A>
where
    P: TurnProvider,
    A: ApprovalPolicy,
{
    pub fn run_turn(
        &mut self,
        input: AgentTurnInput,
        run_log: &mut RunLog,
    ) -> Result<AgentTurnOutcome, AgentTurnLoopError> {
        let turn_id = input.turn_id.clone();
        let result = self.run_turn_inner(input, run_log);
        if let Err(error) = &result
            && !matches!(error, AgentTurnLoopError::RunLog(_))
        {
            let _append_result = run_log.append(
                "run.failed",
                Some(turn_id),
                json!({
                    "code": error.code(),
                    "message": error.to_string(),
                }),
            );
        }

        result
    }

    fn run_turn_inner(
        &mut self,
        input: AgentTurnInput,
        run_log: &mut RunLog,
    ) -> Result<AgentTurnOutcome, AgentTurnLoopError> {
        if self.config.max_model_turns == 0 {
            return Err(AgentTurnLoopError::InvalidConfig {
                detail: "max_model_turns must be greater than zero".to_owned(),
            });
        }

        run_log.append(
            "run.started",
            None,
            json!({
                "runId": run_log.run_id(),
                "workspaceRoot": "workspace",
                "mode": input.mode.as_str(),
            }),
        )?;
        run_log.append(
            "turn.started",
            Some(input.turn_id.clone()),
            json!({
                "turnId": input.turn_id.clone(),
                "userTask": input.user_task.clone(),
            }),
        )?;

        let context = self.build_context(&input)?;
        run_log.append(
            "context.built",
            Some(input.turn_id.clone()),
            context.context_built_payload(),
        )?;

        let mut messages = vec![ChatMessage::user(context.content)];
        let mut tool_results = Vec::new();
        let mut changed_files = Vec::new();

        for iteration in 1..=self.config.max_model_turns {
            let prepared = self.reasoning.prepare_messages(&messages)?;
            run_log.append(
                "provider.requested",
                Some(input.turn_id.clone()),
                json!({
                    "iteration": iteration,
                    "messageCount": prepared.messages.len(),
                    "reasoningState": reasoning_state_payload(prepared.state),
                }),
            )?;

            let response = self.provider.complete(TurnProviderRequest {
                iteration,
                messages: prepared.messages,
            })?;

            if !response.tool_calls.is_empty()
                && self.reasoning.mode() == ReasoningContentMode::ThinkingEnabled
                && response
                    .reasoning_content
                    .as_deref()
                    .is_none_or(|reasoning| reasoning.trim().is_empty())
            {
                return Err(AgentTurnLoopError::MissingAssistantReasoningContent);
            }

            if let Some(content) = response
                .content
                .as_deref()
                .filter(|content| !content.is_empty())
            {
                run_log.append(
                    "assistant.delta",
                    Some(input.turn_id.clone()),
                    json!({
                        "iteration": iteration,
                        "text": content,
                    }),
                )?;
            }

            if response.tool_calls.is_empty() {
                let final_message = response.content.unwrap_or_default();
                run_log.append(
                    "run.completed",
                    Some(input.turn_id.clone()),
                    json!({
                        "summary": final_message,
                        "changedFiles": changed_files.clone(),
                        "verificationStatus": "skipped",
                    }),
                )?;

                return Ok(AgentTurnOutcome {
                    final_message,
                    iterations: iteration,
                    tool_results,
                    changed_files,
                });
            }

            let tool_calls = response.tool_calls;
            messages.push(ChatMessage::assistant_with_tool_calls(
                response.content,
                response.reasoning_content,
                tool_calls.clone(),
            ));

            for (tool_index, tool_call) in tool_calls.iter().enumerate() {
                let executed = self.execute_tool_call(
                    tool_call,
                    &input.turn_id,
                    iteration,
                    tool_index + 1,
                    run_log,
                )?;
                changed_files.extend(executed.changed_files.iter().cloned());
                messages.push(ChatMessage::tool_result(
                    tool_call.id.clone(),
                    executed.message_content.clone(),
                ));
                tool_results.push(AgentToolResult {
                    tool_call_id: tool_call.id.clone(),
                    name: tool_call.function.name.clone(),
                    status: executed.status,
                    result: executed.log_result,
                });
            }
        }

        Err(AgentTurnLoopError::MaxModelTurnsExceeded {
            max_model_turns: self.config.max_model_turns,
        })
    }

    fn build_context(
        &self,
        input: &AgentTurnInput,
    ) -> Result<crate::context::ContextCapsule, AgentTurnLoopError> {
        let mut builder =
            ContextBuilder::new(ContextBuilderConfig::new(self.config.max_input_tokens));
        builder.add_item(ContextItem::user_task(input.user_task.clone()));
        for item in &input.context_items {
            builder.add_item(item.clone());
        }

        Ok(builder.build()?)
    }

    fn execute_tool_call(
        &mut self,
        tool_call: &ChatToolCall,
        turn_id: &str,
        iteration: usize,
        tool_index: usize,
        run_log: &mut RunLog,
    ) -> Result<ExecutedToolCall, AgentTurnLoopError> {
        let tool_name = tool_call.function.name.as_str();
        let definition =
            find_builtin_tool(tool_name).ok_or_else(|| AgentTurnLoopError::UnknownTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            })?;
        let arguments_preview = parse_tool_arguments_value(tool_call)?;

        run_log.append(
            "tool.requested",
            Some(turn_id.to_owned()),
            json!({
                "toolCallId": tool_call.id.clone(),
                "name": tool_name,
                "risk": definition.risk.as_str(),
                "argumentsPreview": arguments_preview,
            }),
        )?;

        match tool_name {
            "read_file" => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.read_file(args)?;
                    tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                })
            }
            "search" => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.search(args)?;
                    tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                })
            }
            "apply_patch" => {
                let args: ApplyPatchArgs = parse_tool_arguments(tool_call)?;
                self.ensure_approval(
                    definition,
                    tool_call,
                    turn_id,
                    iteration,
                    tool_index,
                    Some(args.expected_files.clone()),
                    None,
                    run_log,
                )?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.apply_patch(args)?;
                    tool_record(
                        result.status,
                        result.summary.clone(),
                        result.files.clone(),
                        &result,
                    )
                })
            }
            "shell" => {
                let args: ShellArgs = parse_tool_arguments(tool_call)?;
                self.ensure_approval(
                    definition,
                    tool_call,
                    turn_id,
                    iteration,
                    tool_index,
                    args.cwd.clone().map(|cwd| vec![cwd]),
                    Some(args.command.clone()),
                    run_log,
                )?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.shell(args)?;
                    tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                })
            }
            "git_status" => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.git_status(args)?;
                    tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                })
            }
            "git_diff" => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(tool_call, turn_id, args, run_log, |tools, args| {
                    let result = tools.git_diff(args)?;
                    tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                })
            }
            _ => Err(AgentTurnLoopError::UnsupportedTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            }),
        }
    }

    fn execute_without_approval<Args, F>(
        &self,
        tool_call: &ChatToolCall,
        turn_id: &str,
        args: Args,
        run_log: &mut RunLog,
        execute: F,
    ) -> Result<ExecutedToolCall, AgentTurnLoopError>
    where
        F: FnOnce(&WorkspaceToolExecutor, Args) -> Result<ExecutedToolCall, AgentTurnLoopError>,
    {
        run_log.append(
            "tool.started",
            Some(turn_id.to_owned()),
            json!({
                "toolCallId": tool_call.id.clone(),
                "name": tool_call.function.name.clone(),
            }),
        )?;

        let executed = execute(&self.tools, args)?;
        run_log.append(
            "tool.completed",
            Some(turn_id.to_owned()),
            json!({
                "toolCallId": tool_call.id.clone(),
                "name": tool_call.function.name.clone(),
                "status": executed.status.as_str(),
                "summary": executed.summary.clone(),
                "result": executed.log_result.clone(),
            }),
        )?;

        Ok(executed)
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_approval(
        &mut self,
        definition: &ToolDefinition,
        tool_call: &ChatToolCall,
        turn_id: &str,
        iteration: usize,
        tool_index: usize,
        paths: Option<Vec<String>>,
        command: Option<String>,
        run_log: &mut RunLog,
    ) -> Result<(), AgentTurnLoopError> {
        if definition.approval == ApprovalRequirement::None {
            return Ok(());
        }

        let request = TurnApprovalRequest {
            approval_id: format!("approval_{iteration}_{tool_index}"),
            tool_call_id: tool_call.id.clone(),
            tool_name: definition.name.as_str().to_owned(),
            risk: definition.risk,
            title: approval_title(definition.name.as_str()).to_owned(),
            detail: approval_detail(
                definition.name.as_str(),
                command.as_deref(),
                paths.as_deref(),
            ),
            command,
            paths,
            persistable: definition.approval.is_persistable(),
        };

        run_log.append(
            "tool.approvalRequired",
            Some(turn_id.to_owned()),
            approval_payload(&request),
        )?;

        match self.approval_policy.decide(&request) {
            ApprovalDecision::Approved => Ok(()),
            ApprovalDecision::Rejected { reason } => Err(AgentTurnLoopError::ApprovalRejected {
                approval_id: request.approval_id,
                tool_call_id: request.tool_call_id,
                reason,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentTurnLoopConfig {
    pub max_input_tokens: u64,
    pub max_model_turns: usize,
}

impl Default for AgentTurnLoopConfig {
    fn default() -> Self {
        Self {
            max_input_tokens: 1_000_000,
            max_model_turns: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurnInput {
    pub turn_id: String,
    pub user_task: String,
    pub mode: AgentRunMode,
    pub context_items: Vec<ContextItem>,
}

impl AgentTurnInput {
    pub fn new(turn_id: impl Into<String>, user_task: impl Into<String>) -> Self {
        Self {
            turn_id: turn_id.into(),
            user_task: user_task.into(),
            mode: AgentRunMode::Edit,
            context_items: Vec::new(),
        }
    }

    pub fn with_mode(mut self, mode: AgentRunMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_context_item(mut self, item: ContextItem) -> Self {
        self.context_items.push(item);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunMode {
    Plan,
    Edit,
    Review,
    Ask,
}

impl AgentRunMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Edit => "edit",
            Self::Review => "review",
            Self::Ask => "ask",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurnOutcome {
    pub final_message: String,
    pub iterations: usize,
    pub tool_results: Vec<AgentToolResult>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub status: ToolStatus,
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnProviderRequest {
    pub iteration: usize,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnProviderResponse {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ChatToolCall>,
}

impl TurnProviderResponse {
    pub fn final_text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn tool_calls(
        content: Option<String>,
        reasoning_content: Option<String>,
        tool_calls: Vec<ChatToolCall>,
    ) -> Self {
        Self {
            content,
            reasoning_content,
            tool_calls,
        }
    }
}

pub trait TurnProvider {
    fn complete(
        &mut self,
        request: TurnProviderRequest,
    ) -> Result<TurnProviderResponse, TurnProviderError>;
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct TurnProviderError {
    message: String,
}

impl TurnProviderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnApprovalRequest {
    pub approval_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub risk: RiskLevel,
    pub title: String,
    pub detail: String,
    pub command: Option<String>,
    pub paths: Option<Vec<String>>,
    pub persistable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected { reason: String },
}

pub trait ApprovalPolicy {
    fn decide(&mut self, request: &TurnApprovalRequest) -> ApprovalDecision;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RejectAllApprovalPolicy;

impl ApprovalPolicy for RejectAllApprovalPolicy {
    fn decide(&mut self, request: &TurnApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Rejected {
            reason: format!("approval required for {}", request.tool_name),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AutoApprovePolicy;

impl ApprovalPolicy for AutoApprovePolicy {
    fn decide(&mut self, _request: &TurnApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approved
    }
}

#[derive(Debug, Error)]
pub enum AgentTurnLoopError {
    #[error("invalid turn loop config: {detail}")]
    InvalidConfig { detail: String },
    #[error("context build failed: {0}")]
    ContextBuild(#[from] ContextBuildError),
    #[error("reasoning content state error: {0}")]
    Reasoning(#[from] ReasoningContentError),
    #[error("provider failed: {0}")]
    Provider(#[from] TurnProviderError),
    #[error("run log failed: {0}")]
    RunLog(#[from] RunLogError),
    #[error("tool execution failed: {0}")]
    ToolExecution(#[from] ToolExecutionError),
    #[error("tool call `{tool_call_id}` requested unknown tool `{name}`")]
    UnknownTool { tool_call_id: String, name: String },
    #[error("tool `{name}` is registered but not implemented in the Phase 1 executor")]
    UnsupportedTool { tool_call_id: String, name: String },
    #[error("tool call `{tool_call_id}` for `{name}` has invalid JSON arguments: {source}")]
    InvalidToolArguments {
        tool_call_id: String,
        name: String,
        source: serde_json::Error,
    },
    #[error("tool result serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("assistant tool call response is missing reasoning_content while thinking is enabled")]
    MissingAssistantReasoningContent,
    #[error("approval `{approval_id}` rejected for tool call `{tool_call_id}`: {reason}")]
    ApprovalRejected {
        approval_id: String,
        tool_call_id: String,
        reason: String,
    },
    #[error("model did not finish after {max_model_turns} turns")]
    MaxModelTurnsExceeded { max_model_turns: usize },
}

impl AgentTurnLoopError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidConfig { .. } => "E_INVALID_CONFIG",
            Self::ContextBuild(_) => "E_CONTEXT_BUILD_FAILED",
            Self::Reasoning(_) => "E_REASONING_CONTENT",
            Self::Provider(_) => "E_PROVIDER_ERROR",
            Self::RunLog(_) => "E_RUN_LOG",
            Self::ToolExecution(_) => "E_TOOL_EXECUTION",
            Self::UnknownTool { .. } => "E_UNKNOWN_TOOL",
            Self::UnsupportedTool { .. } => "E_UNSUPPORTED_TOOL",
            Self::InvalidToolArguments { .. } => "E_INVALID_TOOL_ARGUMENTS",
            Self::Serialization(_) => "E_SERIALIZATION",
            Self::MissingAssistantReasoningContent => "E_MISSING_REASONING_CONTENT",
            Self::ApprovalRejected { .. } => "E_APPROVAL_REJECTED",
            Self::MaxModelTurnsExceeded { .. } => "E_MAX_MODEL_TURNS",
        }
    }
}

struct ExecutedToolCall {
    status: ToolStatus,
    summary: String,
    message_content: String,
    log_result: Value,
    changed_files: Vec<String>,
}

fn parse_tool_arguments_value(tool_call: &ChatToolCall) -> Result<Value, AgentTurnLoopError> {
    serde_json::from_str(&tool_call.function.arguments).map_err(|source| {
        AgentTurnLoopError::InvalidToolArguments {
            tool_call_id: tool_call.id.clone(),
            name: tool_call.function.name.clone(),
            source,
        }
    })
}

fn parse_tool_arguments<T: DeserializeOwned>(
    tool_call: &ChatToolCall,
) -> Result<T, AgentTurnLoopError> {
    serde_json::from_str(&tool_call.function.arguments).map_err(|source| {
        AgentTurnLoopError::InvalidToolArguments {
            tool_call_id: tool_call.id.clone(),
            name: tool_call.function.name.clone(),
            source,
        }
    })
}

fn tool_record<T: serde::Serialize>(
    status: ToolStatus,
    summary: String,
    changed_files: Vec<String>,
    result: &T,
) -> Result<ExecutedToolCall, AgentTurnLoopError> {
    let log_result = redacted_tool_result_value(result)?;
    let message_content = serde_json::to_string(&log_result)?;
    Ok(ExecutedToolCall {
        status,
        summary,
        message_content,
        log_result,
        changed_files,
    })
}

fn reasoning_state_payload(state: ReasoningContentState) -> Value {
    match state {
        ReasoningContentState::NoReplayRequired => {
            json!({ "state": "no_replay_required" })
        }
        ReasoningContentState::ReplayRequired { assistant_messages } => {
            json!({
                "state": "replay_required",
                "assistantMessages": assistant_messages,
            })
        }
    }
}

fn approval_title(tool_name: &str) -> &'static str {
    match tool_name {
        "apply_patch" => "Apply patch",
        "shell" => "Run shell command",
        _ => "Approve tool call",
    }
}

fn approval_detail(tool_name: &str, command: Option<&str>, paths: Option<&[String]>) -> String {
    match (tool_name, command, paths) {
        ("shell", Some(command), _) => format!("Execute `{command}`"),
        ("apply_patch", _, Some(paths)) => {
            format!(
                "Modify {} expected file(s): {}",
                paths.len(),
                paths.join(", ")
            )
        }
        _ => format!("Execute tool `{tool_name}`"),
    }
}

fn approval_payload(request: &TurnApprovalRequest) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "approvalId".to_owned(),
        Value::String(request.approval_id.clone()),
    );
    payload.insert(
        "toolCallId".to_owned(),
        Value::String(request.tool_call_id.clone()),
    );
    payload.insert(
        "risk".to_owned(),
        Value::String(request.risk.as_str().to_owned()),
    );
    payload.insert("title".to_owned(), Value::String(request.title.clone()));
    payload.insert("detail".to_owned(), Value::String(request.detail.clone()));
    payload.insert("persistable".to_owned(), Value::Bool(request.persistable));
    if let Some(command) = &request.command {
        payload.insert("command".to_owned(), Value::String(command.clone()));
    }
    if let Some(paths) = &request.paths {
        payload.insert(
            "paths".to_owned(),
            Value::Array(paths.iter().cloned().map(Value::String).collect()),
        );
    }

    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, fs};

    use serde_json::json;

    use crate::{context::ContextItem, provider::deepseek_api::ChatToolCall, run_log::RunLogStore};

    use super::{
        AgentRunMode, AgentTurnInput, AgentTurnLoop, AgentTurnLoopError, AutoApprovePolicy,
        TurnProvider, TurnProviderError, TurnProviderRequest, TurnProviderResponse,
    };

    #[test]
    fn turn_loop_runs_read_tool_and_continues_to_final_answer() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "hello from README\n");
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_read")
            .expect("run should be created");
        let provider = ScriptedProvider::new(vec![
            TurnProviderResponse::tool_calls(
                None,
                Some("I need to inspect the README before answering.".to_owned()),
                vec![ChatToolCall::function(
                    "call_1",
                    "read_file",
                    r#"{"path":"README.md"}"#,
                )],
            ),
            TurnProviderResponse::final_text("README says hello."),
        ]);
        let mut loop_runner =
            AgentTurnLoop::new(workspace.path(), provider).expect("turn loop should initialize");

        let outcome = loop_runner
            .run_turn(
                AgentTurnInput::new("turn_1", "Read README and summarize it")
                    .with_mode(AgentRunMode::Ask)
                    .with_context_item(ContextItem::project_rules(
                        "Answer concisely.",
                        "project instructions",
                    )),
                &mut run,
            )
            .expect("turn should complete");

        assert_eq!(outcome.final_message, "README says hello.");
        assert_eq!(outcome.iterations, 2);
        assert_eq!(outcome.tool_results.len(), 1);
        assert_eq!(loop_runner.provider.requests.len(), 2);
        assert_eq!(
            loop_runner.provider.requests[1].messages[1]
                .reasoning_content
                .as_deref(),
            Some("I need to inspect the README before answering.")
        );
        assert_eq!(
            loop_runner.provider.requests[1].messages[2]
                .content
                .as_deref()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
                .and_then(|value| value["content"].as_str().map(str::to_owned)),
            Some("hello from README\n".to_owned())
        );

        let events = store.load_run("run_turn_read").expect("events should load");
        let event_types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            vec![
                "run.started",
                "turn.started",
                "context.built",
                "provider.requested",
                "tool.requested",
                "tool.started",
                "tool.completed",
                "provider.requested",
                "assistant.delta",
                "run.completed",
            ]
        );
        assert_eq!(events[6].payload["name"], "read_file");
        assert_eq!(
            events[6].payload["result"]["content"],
            "hello from README\n"
        );
    }

    #[test]
    fn turn_loop_requires_approval_for_shell_before_execution() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_reject")
            .expect("run should be created");
        let provider = ScriptedProvider::new(vec![TurnProviderResponse::tool_calls(
            None,
            Some("I need to run a command.".to_owned()),
            vec![ChatToolCall::function(
                "call_1",
                "shell",
                r#"{"command":"Write-Output hello","timeoutMs":1000}"#,
            )],
        )]);
        let mut loop_runner =
            AgentTurnLoop::new(workspace.path(), provider).expect("turn loop should initialize");

        let error = loop_runner
            .run_turn(AgentTurnInput::new("turn_1", "Run a command"), &mut run)
            .expect_err("approval rejection should fail the turn");

        assert!(matches!(error, AgentTurnLoopError::ApprovalRejected { .. }));
        let events = store
            .load_run("run_turn_reject")
            .expect("events should load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.approvalRequired")
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "tool.started")
        );
        assert!(events.iter().any(|event| event.event_type == "run.failed"));
    }

    #[test]
    fn turn_loop_executes_approved_patch_and_tracks_changed_files() {
        let workspace = TestWorkspace::new();
        workspace.write("README.md", "old\n");
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_patch")
            .expect("run should be created");
        let patch = concat!(
            "--- a/README.md\n",
            "+++ b/README.md\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n"
        );
        let provider = ScriptedProvider::new(vec![
            TurnProviderResponse::tool_calls(
                None,
                Some("I should edit the README.".to_owned()),
                vec![ChatToolCall::function(
                    "call_1",
                    "apply_patch",
                    json!({
                        "unifiedDiff": patch,
                        "expectedFiles": ["README.md"],
                    })
                    .to_string(),
                )],
            ),
            TurnProviderResponse::final_text("Updated README."),
        ]);
        let mut loop_runner =
            AgentTurnLoop::with_approval_policy(workspace.path(), provider, AutoApprovePolicy)
                .expect("turn loop should initialize");

        let outcome = loop_runner
            .run_turn(AgentTurnInput::new("turn_1", "Update README"), &mut run)
            .expect("approved patch should complete");

        assert_eq!(workspace.read("README.md"), "new\n");
        assert_eq!(outcome.changed_files, vec!["README.md"]);
        let events = store
            .load_run("run_turn_patch")
            .expect("events should load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.approvalRequired")
        );
        let completed = events
            .iter()
            .find(|event| event.event_type == "run.completed")
            .expect("run should complete");
        assert_eq!(completed.payload["changedFiles"], json!(["README.md"]));
    }

    struct ScriptedProvider {
        responses: VecDeque<TurnProviderResponse>,
        requests: Vec<TurnProviderRequest>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<TurnProviderResponse>) -> Self {
            Self {
                responses: responses.into(),
                requests: Vec::new(),
            }
        }
    }

    impl TurnProvider for ScriptedProvider {
        fn complete(
            &mut self,
            request: TurnProviderRequest,
        ) -> Result<TurnProviderResponse, TurnProviderError> {
            self.requests.push(request);
            self.responses
                .pop_front()
                .ok_or_else(|| TurnProviderError::new("scripted provider has no response"))
        }
    }

    struct TestWorkspace {
        path: std::path::PathBuf,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let unique = format!(
                "deepseek-coder-turn-loop-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock should be after epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp workspace should be created");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
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
