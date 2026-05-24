use std::{future::Future, path::Path, pin::Pin};

use futures_util::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::{
    approval::{ApprovalRequirement, RiskLevel},
    cancellation::{CancellationError, CancellationToken},
    context::{ContextBuildError, ContextBuilder, ContextBuilderConfig, ContextItem},
    provider::deepseek_api::{ChatMessage, ChatToolCall},
    reasoning::{
        ReasoningContentError, ReasoningContentMode, ReasoningContentState,
        ReasoningContentStateMachine,
    },
    run_log::{RunLogError, RunLogEvent, RunLogWriter},
    tool::{ToolDefinition, ToolName, find_builtin_tool},
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
    pub async fn run_turn<L>(
        &mut self,
        input: AgentTurnInput,
        run_log: &mut L,
    ) -> Result<AgentTurnOutcome, AgentTurnLoopError>
    where
        L: RunLogWriter + ?Sized,
    {
        let mut event_sink = NoopTurnEventSink;
        self.run_turn_with_event_sink(input, run_log, &mut event_sink)
            .await
    }

    pub async fn run_turn_with_event_sink<L, S>(
        &mut self,
        input: AgentTurnInput,
        run_log: &mut L,
        event_sink: &mut S,
    ) -> Result<AgentTurnOutcome, AgentTurnLoopError>
    where
        L: RunLogWriter + ?Sized,
        S: TurnEventSink + ?Sized,
    {
        let turn_id = input.turn_id.clone();
        let result = self.run_turn_inner(input, run_log, event_sink).await;
        if let Err(error) = &result
            && !matches!(
                error,
                AgentTurnLoopError::RunLog(_) | AgentTurnLoopError::EventSink(_)
            )
        {
            let (event_type, payload) = terminal_error_event(error);
            let append_result =
                append_turn_event(run_log, event_sink, event_type, Some(turn_id), payload);
            if let Err(append_error) = append_result {
                eprintln!(
                    "failed to append {event_type} event after `{}`: {append_error}",
                    error.code()
                );
            }
        }

        result
    }

    async fn run_turn_inner(
        &mut self,
        input: AgentTurnInput,
        run_log: &mut (impl RunLogWriter + ?Sized),
        event_sink: &mut (impl TurnEventSink + ?Sized),
    ) -> Result<AgentTurnOutcome, AgentTurnLoopError> {
        if self.config.max_model_turns == 0 {
            return Err(AgentTurnLoopError::InvalidConfig {
                detail: "max_model_turns must be greater than zero".to_owned(),
            });
        }
        input.cancellation_token.check()?;

        append_turn_event(
            run_log,
            event_sink,
            "run.started",
            None,
            json!({
                "runId": run_log.run_id(),
                "workspaceRoot": self.tools.root().display().to_string(),
                "mode": input.mode.as_str(),
            }),
        )?;
        append_turn_event(
            run_log,
            event_sink,
            "turn.started",
            Some(input.turn_id.clone()),
            json!({
                "turnId": input.turn_id.clone(),
                "userTask": input.user_task.clone(),
            }),
        )?;

        let context = self.build_context(&input)?;
        append_turn_event(
            run_log,
            event_sink,
            "context.built",
            Some(input.turn_id.clone()),
            context.context_built_payload(),
        )?;

        let mut messages = vec![ChatMessage::user(context.content)];
        let mut tool_results = Vec::new();
        let mut changed_files = Vec::new();

        for iteration in 1..=self.config.max_model_turns {
            let prepared = self.reasoning.prepare_messages(&messages)?;
            append_turn_event(
                run_log,
                event_sink,
                "provider.requested",
                Some(input.turn_id.clone()),
                json!({
                    "iteration": iteration,
                    "messageCount": prepared.messages.len(),
                    "reasoningState": reasoning_state_payload(prepared.state),
                }),
            )?;

            let provider_turn = self
                .collect_provider_response(
                    TurnProviderRequest {
                        iteration,
                        messages: prepared.messages,
                        cancellation_token: input.cancellation_token.clone(),
                    },
                    &input.turn_id,
                    iteration,
                    run_log,
                    event_sink,
                )
                .await?;
            let response = provider_turn.response;

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
                .filter(|_| !provider_turn.emitted_content_delta)
            {
                append_turn_event(
                    run_log,
                    event_sink,
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
                append_turn_event(
                    run_log,
                    event_sink,
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
                let tool_context = ToolCallContext {
                    turn_id: &input.turn_id,
                    iteration,
                    tool_index: tool_index + 1,
                    cancellation_token: &input.cancellation_token,
                };
                let executed =
                    self.execute_tool_call(tool_call, tool_context, run_log, event_sink)?;
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

    async fn collect_provider_response<L>(
        &mut self,
        request: TurnProviderRequest,
        turn_id: &str,
        iteration: usize,
        run_log: &mut L,
        event_sink: &mut (impl TurnEventSink + ?Sized),
    ) -> Result<CollectedProviderTurn, AgentTurnLoopError>
    where
        L: RunLogWriter + ?Sized,
    {
        let cancellation_token = request.cancellation_token.clone();
        cancellation_token.check()?;
        let mut stream = self
            .provider
            .complete_stream(request)
            .await
            .map_err(|error| provider_error_or_canceled(error, &cancellation_token))?;
        cancellation_token.check()?;
        let mut response = None;
        let mut emitted_content_delta = false;

        while let Some(event) = stream.next().await {
            cancellation_token.check()?;
            match event.map_err(|error| provider_error_or_canceled(error, &cancellation_token))? {
                TurnProviderEvent::AssistantDelta(delta) => {
                    if response.is_some() {
                        return Err(AgentTurnLoopError::ProviderEventAfterCompletion);
                    }

                    if let Some(content) = delta
                        .content
                        .as_deref()
                        .filter(|content| !content.is_empty())
                    {
                        emitted_content_delta = true;
                        append_turn_event(
                            run_log,
                            event_sink,
                            "assistant.delta",
                            Some(turn_id.to_owned()),
                            json!({
                                "iteration": iteration,
                                "text": content,
                                "stream": true,
                            }),
                        )?;
                    }
                }
                TurnProviderEvent::Completed(completed) => {
                    if response.replace(completed).is_some() {
                        return Err(AgentTurnLoopError::ProviderCompletedMultipleTimes);
                    }
                }
            }
            cancellation_token.check()?;
        }

        cancellation_token.check()?;
        let response = response.ok_or(AgentTurnLoopError::ProviderStreamEndedWithoutCompletion)?;
        Ok(CollectedProviderTurn {
            response,
            emitted_content_delta,
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
        context: ToolCallContext<'_>,
        run_log: &mut (impl RunLogWriter + ?Sized),
        event_sink: &mut (impl TurnEventSink + ?Sized),
    ) -> Result<ExecutedToolCall, AgentTurnLoopError> {
        context.cancellation_token.check()?;
        let tool_name = tool_call.function.name.as_str();
        let definition =
            find_builtin_tool(tool_name).ok_or_else(|| AgentTurnLoopError::UnknownTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            })?;
        let arguments_preview = parse_tool_arguments_value(tool_call)?;

        append_turn_event(
            run_log,
            event_sink,
            "tool.requested",
            Some(context.turn_id.to_owned()),
            json!({
                "toolCallId": tool_call.id.clone(),
                "name": tool_name,
                "risk": definition.risk.as_str(),
                "argumentsPreview": arguments_preview,
            }),
        )?;

        match definition.name {
            ToolName::WorkspaceManifest => Err(AgentTurnLoopError::UnsupportedTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            }),
            ToolName::ReadFile => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result = tools.read_file_with_cancellation(args, cancellation_token)?;
                        tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                    },
                )
            }
            ToolName::Search => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result = tools.search_with_cancellation(args, cancellation_token)?;
                        tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                    },
                )
            }
            ToolName::ApplyPatch => {
                let args: ApplyPatchArgs = parse_tool_arguments(tool_call)?;
                self.ensure_approval(
                    definition,
                    tool_call,
                    context.turn_id,
                    context.iteration,
                    context.tool_index,
                    Some(args.expected_files.clone()),
                    None,
                    run_log,
                    event_sink,
                )?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result =
                            tools.apply_patch_with_cancellation(args, cancellation_token)?;
                        tool_record(
                            result.status,
                            result.summary.clone(),
                            result.files.clone(),
                            &result,
                        )
                    },
                )
            }
            ToolName::Shell => {
                let args: ShellArgs = parse_tool_arguments(tool_call)?;
                self.ensure_approval(
                    definition,
                    tool_call,
                    context.turn_id,
                    context.iteration,
                    context.tool_index,
                    args.cwd.clone().map(|cwd| vec![cwd]),
                    Some(args.command.clone()),
                    run_log,
                    event_sink,
                )?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result = tools.shell_with_cancellation(args, cancellation_token)?;
                        tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                    },
                )
            }
            ToolName::GitStatus => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result =
                            tools.git_status_with_cancellation(args, cancellation_token)?;
                        tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                    },
                )
            }
            ToolName::GitDiff => {
                let args = parse_tool_arguments(tool_call)?;
                self.execute_without_approval(
                    tool_call,
                    context,
                    args,
                    run_log,
                    event_sink,
                    |tools, args, cancellation_token| {
                        let result = tools.git_diff_with_cancellation(args, cancellation_token)?;
                        tool_record(result.status, result.summary.clone(), Vec::new(), &result)
                    },
                )
            }
            ToolName::LspDiagnostics => Err(AgentTurnLoopError::UnsupportedTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            }),
            ToolName::PlanUpdate => Err(AgentTurnLoopError::UnsupportedTool {
                tool_call_id: tool_call.id.clone(),
                name: tool_name.to_owned(),
            }),
        }
    }

    fn execute_without_approval<L, Args, F>(
        &self,
        tool_call: &ChatToolCall,
        context: ToolCallContext<'_>,
        args: Args,
        run_log: &mut L,
        event_sink: &mut (impl TurnEventSink + ?Sized),
        execute: F,
    ) -> Result<ExecutedToolCall, AgentTurnLoopError>
    where
        L: RunLogWriter + ?Sized,
        F: FnOnce(
            &WorkspaceToolExecutor,
            Args,
            &CancellationToken,
        ) -> Result<ExecutedToolCall, AgentTurnLoopError>,
    {
        context.cancellation_token.check()?;
        append_turn_event(
            run_log,
            event_sink,
            "tool.started",
            Some(context.turn_id.to_owned()),
            json!({
                "toolCallId": tool_call.id.clone(),
                "name": tool_call.function.name.clone(),
            }),
        )?;

        let executed = execute(&self.tools, args, context.cancellation_token)?;
        append_turn_event(
            run_log,
            event_sink,
            "tool.completed",
            Some(context.turn_id.to_owned()),
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
        run_log: &mut (impl RunLogWriter + ?Sized),
        event_sink: &mut (impl TurnEventSink + ?Sized),
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

        append_turn_event(
            run_log,
            event_sink,
            "tool.approvalRequired",
            Some(turn_id.to_owned()),
            approval_payload(&request),
        )?;

        match self.approval_policy.decide(&request)? {
            ApprovalDecision::Approved => {
                append_turn_event(
                    run_log,
                    event_sink,
                    "tool.approvalResolved",
                    Some(turn_id.to_owned()),
                    approval_resolved_payload(&request, "approved", None),
                )?;
                Ok(())
            }
            ApprovalDecision::Rejected { reason } => {
                append_turn_event(
                    run_log,
                    event_sink,
                    "tool.approvalResolved",
                    Some(turn_id.to_owned()),
                    approval_resolved_payload(&request, "rejected", Some(reason.as_str())),
                )?;
                Err(AgentTurnLoopError::ApprovalRejected {
                    approval_id: request.approval_id,
                    tool_call_id: request.tool_call_id,
                    reason,
                })
            }
            ApprovalDecision::Canceled { reason } => {
                append_turn_event(
                    run_log,
                    event_sink,
                    "tool.approvalResolved",
                    Some(turn_id.to_owned()),
                    approval_resolved_payload(&request, "canceled", Some(reason.as_str())),
                )?;
                Err(AgentTurnLoopError::ApprovalCanceled {
                    approval_id: request.approval_id,
                    tool_call_id: request.tool_call_id,
                    reason,
                })
            }
            ApprovalDecision::Expired { reason } => {
                append_turn_event(
                    run_log,
                    event_sink,
                    "tool.approvalResolved",
                    Some(turn_id.to_owned()),
                    approval_resolved_payload(&request, "expired", Some(reason.as_str())),
                )?;
                Err(AgentTurnLoopError::ApprovalExpired {
                    approval_id: request.approval_id,
                    tool_call_id: request.tool_call_id,
                    reason,
                })
            }
        }
    }
}

pub trait TurnEventSink {
    fn on_event(&mut self, event: &RunLogEvent) -> Result<(), TurnEventSinkError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTurnEventSink;

impl TurnEventSink for NoopTurnEventSink {
    fn on_event(&mut self, _event: &RunLogEvent) -> Result<(), TurnEventSinkError> {
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("turn event sink failed: {message}")]
pub struct TurnEventSinkError {
    message: String,
}

impl TurnEventSinkError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn append_turn_event<L>(
    run_log: &mut L,
    event_sink: &mut (impl TurnEventSink + ?Sized),
    event_type: impl Into<String>,
    turn_id: Option<String>,
    payload: Value,
) -> Result<RunLogEvent, AgentTurnLoopError>
where
    L: RunLogWriter + ?Sized,
{
    let event = run_log.append_event(event_type.into(), turn_id, payload)?;
    event_sink.on_event(&event)?;
    Ok(event)
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
    pub cancellation_token: CancellationToken,
}

impl AgentTurnInput {
    pub fn new(turn_id: impl Into<String>, user_task: impl Into<String>) -> Self {
        Self {
            turn_id: turn_id.into(),
            user_task: user_task.into(),
            mode: AgentRunMode::Edit,
            context_items: Vec::new(),
            cancellation_token: CancellationToken::new(),
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

    pub fn with_cancellation_token(mut self, cancellation_token: CancellationToken) -> Self {
        self.cancellation_token = cancellation_token;
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
    pub cancellation_token: CancellationToken,
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

#[derive(Debug, Clone, PartialEq)]
pub enum TurnProviderEvent {
    AssistantDelta(TurnProviderDelta),
    Completed(TurnProviderResponse),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnProviderDelta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

impl TurnProviderDelta {
    pub fn new(content: Option<String>, reasoning_content: Option<String>) -> Self {
        Self {
            content,
            reasoning_content,
        }
    }

    pub fn content(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            reasoning_content: None,
        }
    }

    pub fn reasoning_content(reasoning_content: impl Into<String>) -> Self {
        Self {
            content: None,
            reasoning_content: Some(reasoning_content.into()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content
            .as_deref()
            .is_none_or(|content| content.is_empty())
            && self
                .reasoning_content
                .as_deref()
                .is_none_or(|reasoning_content| reasoning_content.is_empty())
    }
}

pub type TurnProviderStream =
    Pin<Box<dyn Stream<Item = Result<TurnProviderEvent, TurnProviderError>> + Send>>;

pub type TurnProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TurnProviderStream, TurnProviderError>> + Send + 'a>>;

pub fn turn_provider_response_stream(response: TurnProviderResponse) -> TurnProviderStream {
    Box::pin(futures_util::stream::once(async move {
        Ok(TurnProviderEvent::Completed(response))
    }))
}

pub trait TurnProvider {
    fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_>;
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
    Canceled { reason: String },
    Expired { reason: String },
}

pub trait ApprovalPolicy {
    fn decide(
        &mut self,
        request: &TurnApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalPolicyError>;
}

#[derive(Debug, Error)]
#[error("approval policy failed: {message}")]
pub struct ApprovalPolicyError {
    message: String,
}

impl ApprovalPolicyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for ApprovalPolicyError {
    fn from(source: std::io::Error) -> Self {
        Self::new(source.to_string())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RejectAllApprovalPolicy;

impl ApprovalPolicy for RejectAllApprovalPolicy {
    fn decide(
        &mut self,
        request: &TurnApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalPolicyError> {
        Ok(ApprovalDecision::Rejected {
            reason: format!("approval required for {}", request.tool_name),
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AutoApprovePolicy;

impl ApprovalPolicy for AutoApprovePolicy {
    fn decide(
        &mut self,
        _request: &TurnApprovalRequest,
    ) -> Result<ApprovalDecision, ApprovalPolicyError> {
        Ok(ApprovalDecision::Approved)
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
    #[error("provider stream ended without a completed response")]
    ProviderStreamEndedWithoutCompletion,
    #[error("provider stream emitted more than one completed response")]
    ProviderCompletedMultipleTimes,
    #[error("provider stream emitted an event after the completed response")]
    ProviderEventAfterCompletion,
    #[error("run log failed: {0}")]
    RunLog(#[from] RunLogError),
    #[error("event sink failed: {0}")]
    EventSink(#[from] TurnEventSinkError),
    #[error("tool execution failed: {0}")]
    ToolExecution(ToolExecutionError),
    #[error("run canceled: {reason}")]
    Canceled { reason: String },
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
    #[error("approval `{approval_id}` canceled for tool call `{tool_call_id}`: {reason}")]
    ApprovalCanceled {
        approval_id: String,
        tool_call_id: String,
        reason: String,
    },
    #[error("approval `{approval_id}` expired for tool call `{tool_call_id}`: {reason}")]
    ApprovalExpired {
        approval_id: String,
        tool_call_id: String,
        reason: String,
    },
    #[error("approval policy failed: {0}")]
    ApprovalPolicy(#[from] ApprovalPolicyError),
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
            Self::ProviderStreamEndedWithoutCompletion => "E_PROVIDER_STREAM_INCOMPLETE",
            Self::ProviderCompletedMultipleTimes => "E_PROVIDER_STREAM_INVALID",
            Self::ProviderEventAfterCompletion => "E_PROVIDER_STREAM_INVALID",
            Self::RunLog(_) => "E_RUN_LOG",
            Self::EventSink(_) => "E_EVENT_SINK",
            Self::ToolExecution(_) => "E_TOOL_EXECUTION",
            Self::Canceled { .. } => "E_RUN_CANCELED",
            Self::UnknownTool { .. } => "E_UNKNOWN_TOOL",
            Self::UnsupportedTool { .. } => "E_UNSUPPORTED_TOOL",
            Self::InvalidToolArguments { .. } => "E_INVALID_TOOL_ARGUMENTS",
            Self::Serialization(_) => "E_SERIALIZATION",
            Self::MissingAssistantReasoningContent => "E_MISSING_REASONING_CONTENT",
            Self::ApprovalRejected { .. } => "E_APPROVAL_REJECTED",
            Self::ApprovalCanceled { .. } => "E_APPROVAL_CANCELED",
            Self::ApprovalExpired { .. } => "E_APPROVAL_EXPIRED",
            Self::ApprovalPolicy(_) => "E_APPROVAL_POLICY",
            Self::MaxModelTurnsExceeded { .. } => "E_MAX_MODEL_TURNS",
        }
    }
}

impl From<ToolExecutionError> for AgentTurnLoopError {
    fn from(error: ToolExecutionError) -> Self {
        match error {
            ToolExecutionError::CommandCanceled { reason, .. } => Self::Canceled { reason },
            error => Self::ToolExecution(error),
        }
    }
}

impl From<CancellationError> for AgentTurnLoopError {
    fn from(error: CancellationError) -> Self {
        Self::Canceled {
            reason: error.reason().to_owned(),
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

struct CollectedProviderTurn {
    // `response` is the authoritative completed assistant message used for final text,
    // reasoning replay, and tool execution. Streaming deltas are only presentation/log events.
    response: TurnProviderResponse,
    // Only visible content deltas count here. Reasoning deltas stay provider-private, and this
    // flag prevents duplicating the final content as another assistant.delta after streaming.
    emitted_content_delta: bool,
}

#[derive(Clone, Copy)]
struct ToolCallContext<'a> {
    turn_id: &'a str,
    iteration: usize,
    tool_index: usize,
    cancellation_token: &'a CancellationToken,
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
        "toolName".to_owned(),
        Value::String(request.tool_name.clone()),
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

fn approval_resolved_payload(
    request: &TurnApprovalRequest,
    decision: &'static str,
    reason: Option<&str>,
) -> Value {
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
        "toolName".to_owned(),
        Value::String(request.tool_name.clone()),
    );
    payload.insert("decision".to_owned(), Value::String(decision.to_owned()));
    if let Some(reason) = reason {
        payload.insert("reason".to_owned(), Value::String(reason.to_owned()));
    }

    Value::Object(payload)
}

fn provider_error_or_canceled(
    error: TurnProviderError,
    cancellation_token: &CancellationToken,
) -> AgentTurnLoopError {
    if cancellation_token.is_canceled() {
        AgentTurnLoopError::Canceled {
            reason: cancellation_token.cancellation_reason(),
        }
    } else {
        AgentTurnLoopError::Provider(error)
    }
}

fn terminal_error_event(error: &AgentTurnLoopError) -> (&'static str, Value) {
    match error {
        AgentTurnLoopError::ApprovalCanceled {
            approval_id,
            tool_call_id,
            reason,
        }
        | AgentTurnLoopError::ApprovalExpired {
            approval_id,
            tool_call_id,
            reason,
        } => (
            "run.canceled",
            json!({
                "code": error.code(),
                "message": error.to_string(),
                "approvalId": approval_id,
                "toolCallId": tool_call_id,
                "reason": reason,
            }),
        ),
        AgentTurnLoopError::Canceled { reason } => (
            "run.canceled",
            json!({
                "code": error.code(),
                "message": error.to_string(),
                "reason": reason,
            }),
        ),
        _ => (
            "run.failed",
            json!({
                "code": error.code(),
                "message": error.to_string(),
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use futures_util::stream;
    use serde_json::json;

    use crate::{
        context::ContextItem,
        provider::deepseek_api::ChatToolCall,
        run_log::{RunLogEvent, RunLogStore},
    };

    use super::{
        AgentRunMode, AgentTurnInput, AgentTurnLoop, AgentTurnLoopError, AutoApprovePolicy,
        CancellationToken, TurnEventSink, TurnEventSinkError, TurnProvider, TurnProviderDelta,
        TurnProviderError, TurnProviderEvent, TurnProviderFuture, TurnProviderRequest,
        TurnProviderResponse, TurnProviderStream, turn_provider_response_stream,
    };

    static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(1);

    #[tokio::test]
    async fn turn_loop_runs_read_tool_and_continues_to_final_answer() {
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
            .await
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
        let workspace_root = fs::canonicalize(workspace.path())
            .expect("workspace path should canonicalize")
            .display()
            .to_string();
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
        assert_eq!(events[0].payload["workspaceRoot"], workspace_root);
        assert_eq!(events[6].payload["name"], "read_file");
        assert_eq!(
            events[6].payload["result"]["content"],
            "hello from README\n"
        );
    }

    #[tokio::test]
    async fn turn_loop_requires_approval_for_shell_before_execution() {
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
            .await
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

    #[tokio::test]
    async fn turn_loop_executes_approved_patch_and_tracks_changed_files() {
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
            .await
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

    #[tokio::test]
    async fn turn_loop_logs_streamed_content_deltas_once() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_streaming")
            .expect("run should be created");
        let provider = EventScriptedProvider::new(vec![vec![
            TurnProviderEvent::AssistantDelta(TurnProviderDelta::content("hello ")),
            TurnProviderEvent::AssistantDelta(TurnProviderDelta::reasoning_content(
                "private reasoning",
            )),
            TurnProviderEvent::AssistantDelta(TurnProviderDelta::content("world")),
            TurnProviderEvent::Completed(TurnProviderResponse::final_text("hello world")),
        ]]);
        let mut loop_runner =
            AgentTurnLoop::new(workspace.path(), provider).expect("turn loop should initialize");

        let outcome = loop_runner
            .run_turn(AgentTurnInput::new("turn_1", "Say hello"), &mut run)
            .await
            .expect("streaming turn should complete");

        assert_eq!(outcome.final_message, "hello world");
        let events = store
            .load_run("run_turn_streaming")
            .expect("events should load");
        let deltas = events
            .iter()
            .filter(|event| event.event_type == "assistant.delta")
            .collect::<Vec<_>>();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].payload["text"], "hello ");
        assert_eq!(deltas[0].payload["stream"], true);
        assert_eq!(deltas[1].payload["text"], "world");
    }

    #[tokio::test]
    async fn turn_loop_sends_each_persisted_event_to_sink() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_sink")
            .expect("run should be created");
        let provider = EventScriptedProvider::new(vec![vec![
            TurnProviderEvent::AssistantDelta(TurnProviderDelta::content("live")),
            TurnProviderEvent::Completed(TurnProviderResponse::final_text("live")),
        ]]);
        let mut loop_runner =
            AgentTurnLoop::new(workspace.path(), provider).expect("turn loop should initialize");
        let mut sink = RecordingEventSink::default();

        let outcome = loop_runner
            .run_turn_with_event_sink(
                AgentTurnInput::new("turn_1", "Say hello"),
                &mut run,
                &mut sink,
            )
            .await
            .expect("streaming turn should complete");

        assert_eq!(outcome.final_message, "live");
        let events = store.load_run("run_turn_sink").expect("events should load");
        assert_eq!(sink.events, events);
        assert!(
            sink.events
                .iter()
                .any(|event| event.event_type == "assistant.delta")
        );
    }

    #[tokio::test]
    async fn turn_loop_cancels_provider_stream_when_token_is_signaled() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_provider_cancel")
            .expect("run should be created");
        let cancellation_token = CancellationToken::new();
        let provider = EventScriptedProvider::new(vec![vec![
            TurnProviderEvent::AssistantDelta(TurnProviderDelta::content("partial")),
            TurnProviderEvent::Completed(TurnProviderResponse::final_text("complete")),
        ]]);
        let mut loop_runner =
            AgentTurnLoop::new(workspace.path(), provider).expect("turn loop should initialize");
        let mut sink = CancelOnEventSink::new(
            cancellation_token.clone(),
            "assistant.delta",
            "provider canceled by test",
        );

        let error = loop_runner
            .run_turn_with_event_sink(
                AgentTurnInput::new("turn_1", "Cancel during provider stream")
                    .with_cancellation_token(cancellation_token),
                &mut run,
                &mut sink,
            )
            .await
            .expect_err("turn should be canceled");

        assert!(matches!(error, AgentTurnLoopError::Canceled { .. }));
        let events = store
            .load_run("run_turn_provider_cancel")
            .expect("events should load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "assistant.delta")
        );
        assert!(events.iter().any(|event| {
            event.event_type == "run.canceled"
                && event.payload["code"] == "E_RUN_CANCELED"
                && event.payload["reason"] == "provider canceled by test"
        }));
    }

    #[tokio::test]
    async fn turn_loop_cancels_shell_tool_when_token_is_signaled() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("run log store should open");
        let mut run = store
            .create_run("run_turn_tool_cancel")
            .expect("run should be created");
        let cancellation_token = CancellationToken::new();
        #[cfg(windows)]
        let command = "Start-Sleep -Seconds 5; Write-Output done";
        #[cfg(not(windows))]
        let command = "sleep 5; printf done";
        let provider = ScriptedProvider::new(vec![TurnProviderResponse::tool_calls(
            None,
            Some("Run a long command.".to_owned()),
            vec![ChatToolCall::function(
                "call_shell",
                "shell",
                json!({
                    "command": command,
                    "cwd": null,
                    "timeoutMs": 10_000
                })
                .to_string(),
            )],
        )]);
        let mut loop_runner =
            AgentTurnLoop::with_approval_policy(workspace.path(), provider, AutoApprovePolicy)
                .expect("turn loop should initialize");
        let mut sink = CancelOnEventSink::new(
            cancellation_token.clone(),
            "tool.started",
            "tool canceled by test",
        );

        let error = loop_runner
            .run_turn_with_event_sink(
                AgentTurnInput::new("turn_1", "Cancel shell")
                    .with_cancellation_token(cancellation_token),
                &mut run,
                &mut sink,
            )
            .await
            .expect_err("turn should be canceled");

        assert!(matches!(error, AgentTurnLoopError::Canceled { .. }));
        let events = store
            .load_run("run_turn_tool_cancel")
            .expect("events should load");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool.started")
        );
        assert!(events.iter().any(|event| {
            event.event_type == "run.canceled"
                && event.payload["code"] == "E_RUN_CANCELED"
                && event.payload["reason"] == "tool canceled by test"
        }));
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
        fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_> {
            Box::pin(async move {
                self.requests.push(request);
                let response = self
                    .responses
                    .pop_front()
                    .ok_or_else(|| TurnProviderError::new("scripted provider has no response"))?;
                Ok(turn_provider_response_stream(response))
            })
        }
    }

    struct EventScriptedProvider {
        streams: VecDeque<Vec<TurnProviderEvent>>,
    }

    impl EventScriptedProvider {
        fn new(streams: Vec<Vec<TurnProviderEvent>>) -> Self {
            Self {
                streams: streams.into(),
            }
        }
    }

    impl TurnProvider for EventScriptedProvider {
        fn complete_stream(&mut self, _request: TurnProviderRequest) -> TurnProviderFuture<'_> {
            Box::pin(async move {
                let events = self.streams.pop_front().ok_or_else(|| {
                    TurnProviderError::new("event scripted provider has no stream")
                })?;
                let stream: TurnProviderStream = Box::pin(stream::iter(events.into_iter().map(Ok)));
                Ok(stream)
            })
        }
    }

    #[derive(Default)]
    struct RecordingEventSink {
        events: Vec<RunLogEvent>,
    }

    impl TurnEventSink for RecordingEventSink {
        fn on_event(&mut self, event: &RunLogEvent) -> Result<(), TurnEventSinkError> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    struct CancelOnEventSink {
        cancellation_token: CancellationToken,
        event_type: &'static str,
        reason: &'static str,
    }

    impl CancelOnEventSink {
        fn new(
            cancellation_token: CancellationToken,
            event_type: &'static str,
            reason: &'static str,
        ) -> Self {
            Self {
                cancellation_token,
                event_type,
                reason,
            }
        }
    }

    impl TurnEventSink for CancelOnEventSink {
        fn on_event(&mut self, event: &RunLogEvent) -> Result<(), TurnEventSinkError> {
            if event.event_type == self.event_type {
                self.cancellation_token.cancel(self.reason);
            }
            Ok(())
        }
    }

    struct TestWorkspace {
        path: std::path::PathBuf,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed);
            let unique = format!(
                "deepseek-coder-turn-loop-test-{}-{}-{}",
                std::process::id(),
                id,
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
