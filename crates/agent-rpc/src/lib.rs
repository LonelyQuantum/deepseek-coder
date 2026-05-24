#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};

use deepseek_coder_agent_core::{
    AGENT_METADATA,
    approval::ALL_RISK_LEVELS,
    run_log::RunLogEvent,
    turn_loop::{TurnEventSink, TurnEventSinkError},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

pub const JSON_RPC_VERSION: &str = "2.0";
pub const PROTOCOL_VERSION: &str = "0.1.0";
pub const METHOD_NAMESPACE: &str = "agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcMethod {
    pub name: &'static str,
}

impl RpcMethod {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub fn qualified_name(self) -> String {
        format!("{METHOD_NAMESPACE}.{}", self.name)
    }
}

pub const INITIALIZE_METHOD: RpcMethod = RpcMethod::new("initialize");
pub const SEND_TURN_METHOD: RpcMethod = RpcMethod::new("sendTurn");
pub const APPROVE_METHOD: RpcMethod = RpcMethod::new("approve");
pub const REJECT_METHOD: RpcMethod = RpcMethod::new("reject");
pub const CANCEL_METHOD: RpcMethod = RpcMethod::new("cancel");
pub const RESUME_METHOD: RpcMethod = RpcMethod::new("resume");
pub const LIST_RUNS_METHOD: RpcMethod = RpcMethod::new("listRuns");
pub const EVENT_METHOD: RpcMethod = RpcMethod::new("event");

pub const JSON_RPC_PARSE_ERROR: i64 = -32700;
pub const JSON_RPC_INVALID_REQUEST: i64 = -32600;
pub const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
pub const JSON_RPC_INVALID_PARAMS: i64 = -32602;
pub const JSON_RPC_INTERNAL_ERROR: i64 = -32603;

pub const RPC_UNSUPPORTED_PROTOCOL: i64 = -32001;
pub const RPC_WORKSPACE_UNTRUSTED: i64 = -32002;
pub const RPC_RUN_NOT_FOUND: i64 = -32003;
pub const RPC_RUN_ALREADY_ACTIVE: i64 = -32004;
pub const RPC_APPROVAL_NOT_FOUND: i64 = -32011;
pub const RPC_APPROVAL_DENIED: i64 = -32012;
pub const RPC_RUN_CANCELED: i64 = -32050;
pub const RPC_INTERNAL_INVARIANT: i64 = -32060;

pub const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest<TParams = Value> {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<TParams>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse<TResult = Value> {
    pub jsonrpc: String,
    pub id: Value,
    pub result: TResult,
}

impl<TResult> JsonRpcResponse<TResult> {
    pub fn new(id: Value, result: TResult) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id,
            result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub error: JsonRpcErrorObject,
}

impl JsonRpcErrorResponse {
    pub fn new(id: Value, error: JsonRpcErrorObject) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id,
            error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcErrorObject {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrontendKind {
    Cli,
    Tui,
    Vscode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    pub frontend: FrontendKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInitializeParams {
    pub protocol_version: String,
    pub client: ClientInfo,
    pub workspace_root: String,
    pub workspace_trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub protocol_version: String,
    pub supports_run_resume: bool,
    pub supports_patch_approval: bool,
    pub supports_persistent_approvals: bool,
    pub supported_risk_levels: Vec<String>,
}

impl Default for ServerCapabilities {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            supports_run_resume: true,
            supports_patch_approval: true,
            supports_persistent_approvals: false,
            supported_risk_levels: ALL_RISK_LEVELS
                .iter()
                .map(|risk| risk.as_str().to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInitializeResult {
    pub protocol_version: String,
    pub server: ServerInfo,
    pub capabilities: ServerCapabilities,
    pub state_dir: String,
}

impl Default for AgentInitializeResult {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            server: ServerInfo {
                name: "deepseek-coder-agent-rpc".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: ServerCapabilities::default(),
            state_dir: AGENT_METADATA.state_dir.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpcRunMode {
    Plan,
    Edit,
    Review,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTurnParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub message: String,
    pub mode: RpcRunMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<TurnAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAttachment {
    pub kind: TurnAttachmentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<TextRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnAttachmentKind {
    File,
    Selection,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRange {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTurnResult {
    pub run_id: String,
    pub turn_id: String,
    pub accepted: bool,
}

impl From<RpcRunMode> for AgentRunMode {
    fn from(value: RpcRunMode) -> Self {
        match value {
            RpcRunMode::Plan => Self::Plan,
            RpcRunMode::Edit => Self::Edit,
            RpcRunMode::Review => Self::Review,
            RpcRunMode::Ask => Self::Ask,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeParams {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_from_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeResult {
    pub run_id: String,
    pub next_seq: u64,
    pub replay_started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcApprovalPersistence {
    Never,
    Session,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcApprovalState {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcRunState {
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveParams {
    pub approval_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persist: Option<RpcApprovalPersistence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveResult {
    pub approval_id: String,
    pub state: RpcApprovalState,
    pub persist: RpcApprovalPersistence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectParams {
    pub approval_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectResult {
    pub approval_id: String,
    pub state: RpcApprovalState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelParams {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelResult {
    pub run_id: String,
    pub state: RpcRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRpcHandlerOutput<TResult> {
    pub result: TResult,
    pub events: Vec<RunLogEvent>,
}

impl<TResult> AgentRpcHandlerOutput<TResult> {
    pub fn new(result: TResult) -> Self {
        Self {
            result,
            events: Vec::new(),
        }
    }

    pub fn with_events(mut self, events: Vec<RunLogEvent>) -> Self {
        self.events = events;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRpcHandlerError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl AgentRpcHandlerError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    fn into_error_object(self) -> JsonRpcErrorObject {
        JsonRpcErrorObject {
            code: self.code,
            message: self.message,
            data: self.data,
        }
    }
}

pub trait AgentRpcRequestHandler {
    fn initialize(
        &mut self,
        params: AgentInitializeParams,
    ) -> Result<AgentInitializeResult, AgentRpcHandlerError>;

    fn send_turn(
        &mut self,
        params: SendTurnParams,
    ) -> Result<AgentRpcHandlerOutput<SendTurnResult>, AgentRpcHandlerError>;

    fn approve(
        &mut self,
        params: ApproveParams,
    ) -> Result<AgentRpcHandlerOutput<ApproveResult>, AgentRpcHandlerError>;

    fn reject(
        &mut self,
        params: RejectParams,
    ) -> Result<AgentRpcHandlerOutput<RejectResult>, AgentRpcHandlerError>;

    fn resume(
        &mut self,
        params: ResumeParams,
    ) -> Result<AgentRpcHandlerOutput<ResumeResult>, AgentRpcHandlerError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcNotification<TParams> {
    pub jsonrpc: String,
    pub method: String,
    pub params: TParams,
}

impl<TParams> JsonRpcNotification<TParams> {
    pub fn new(method: RpcMethod, params: TParams) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            method: method.qualified_name(),
            params,
        }
    }
}

pub type AgentEventNotification = JsonRpcNotification<AgentEventEnvelope>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventEnvelope {
    pub seq: u64,
    pub time: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub payload: Value,
}

pub fn run_log_event_to_envelope(event: &RunLogEvent) -> Result<AgentEventEnvelope, AgentRpcError> {
    Ok(AgentEventEnvelope {
        seq: event.seq,
        time: format_unix_millis(event.time_unix_ms)?,
        event_type: event.event_type.clone(),
        run_id: event.run_id.clone(),
        turn_id: event.turn_id.clone(),
        payload: event.payload.clone(),
    })
}

pub fn run_log_event_to_notification(
    event: &RunLogEvent,
) -> Result<AgentEventNotification, AgentRpcError> {
    Ok(JsonRpcNotification::new(
        EVENT_METHOD,
        run_log_event_to_envelope(event)?,
    ))
}

#[derive(Debug)]
pub struct StdioEventBridge<W> {
    writer: W,
}

impl<W> StdioEventBridge<W>
where
    W: Write,
{
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn emit_event(&mut self, event: &RunLogEvent) -> Result<AgentEventEnvelope, AgentRpcError> {
        let notification = run_log_event_to_notification(event)?;
        let envelope = notification.params.clone();
        write_json_line(&mut self.writer, &notification)?;
        Ok(envelope)
    }

    pub fn emit_events<'event>(
        &mut self,
        events: impl IntoIterator<Item = &'event RunLogEvent>,
    ) -> Result<Vec<AgentEventEnvelope>, AgentRpcError> {
        events
            .into_iter()
            .map(|event| self.emit_event(event))
            .collect()
    }
}

impl<W> TurnEventSink for StdioEventBridge<W>
where
    W: Write,
{
    fn on_event(&mut self, event: &RunLogEvent) -> Result<(), TurnEventSinkError> {
        self.emit_event(event)
            .map(|_| ())
            .map_err(|source| TurnEventSinkError::new(source.to_string()))
    }
}

#[derive(Debug)]
pub struct AgentRpcServer<H> {
    handler: H,
    initialized: bool,
}

impl<H> AgentRpcServer<H>
where
    H: AgentRpcRequestHandler,
{
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            initialized: false,
        }
    }

    pub fn into_inner(self) -> H {
        self.handler
    }

    pub fn handle_line<W>(&mut self, line: &str, writer: &mut W) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        if line.trim().is_empty() {
            return Ok(());
        }

        let message = match parse_incoming_message(line) {
            Ok(message) => message,
            Err(error) => {
                let id = error.id.clone();
                write_json_line(
                    writer,
                    &JsonRpcErrorResponse::new(id, error.into_error_object()),
                )?;
                return Ok(());
            }
        };

        let Some(id) = message.id else {
            return Ok(());
        };

        if message.jsonrpc != JSON_RPC_VERSION {
            return write_error(
                writer,
                id,
                JsonRpcErrorObject::new(
                    JSON_RPC_INVALID_REQUEST,
                    "JSON-RPC message must use version 2.0",
                ),
            );
        }

        if !self.initialized && message.method != INITIALIZE_METHOD.qualified_name() {
            return write_error(
                writer,
                id,
                JsonRpcErrorObject::new(
                    JSON_RPC_INVALID_REQUEST,
                    "agent.initialize must be the first request",
                ),
            );
        }

        match message.method.as_str() {
            method if method == INITIALIZE_METHOD.qualified_name() => {
                self.handle_initialize(id, message.params, writer)
            }
            method if method == SEND_TURN_METHOD.qualified_name() => {
                self.handle_send_turn(id, message.params, writer)
            }
            method if method == APPROVE_METHOD.qualified_name() => {
                self.handle_approve(id, message.params, writer)
            }
            method if method == REJECT_METHOD.qualified_name() => {
                self.handle_reject(id, message.params, writer)
            }
            method if method == CANCEL_METHOD.qualified_name() => {
                self.handle_cancel(id, message.params, writer)
            }
            method if method == RESUME_METHOD.qualified_name() => {
                self.handle_resume(id, message.params, writer)
            }
            method => write_error(
                writer,
                id,
                JsonRpcErrorObject::new(
                    JSON_RPC_METHOD_NOT_FOUND,
                    format!("method `{method}` is not supported by this request loop"),
                ),
            ),
        }
    }

    fn handle_initialize<W>(
        &mut self,
        id: Value,
        params: Option<Value>,
        writer: &mut W,
    ) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        if self.initialized {
            return write_error(
                writer,
                id,
                JsonRpcErrorObject::new(
                    JSON_RPC_INVALID_REQUEST,
                    "agent.initialize must not be called more than once",
                ),
            );
        }

        let params = match parse_params::<AgentInitializeParams>(
            params,
            INITIALIZE_METHOD.qualified_name().as_str(),
        ) {
            Ok(params) => params,
            Err(error) => return write_error(writer, id, error),
        };

        if params.protocol_version != PROTOCOL_VERSION {
            return write_error(
                writer,
                id,
                JsonRpcErrorObject::new(
                    RPC_UNSUPPORTED_PROTOCOL,
                    format!(
                        "unsupported protocol version `{}`, expected `{PROTOCOL_VERSION}`",
                        params.protocol_version
                    ),
                )
                .with_data(json!({
                    "clientProtocolVersion": params.protocol_version,
                    "serverProtocolVersion": PROTOCOL_VERSION,
                })),
            );
        }

        match self.handler.initialize(params) {
            Ok(result) => {
                write_json_line(writer, &JsonRpcResponse::new(id, result))?;
                self.initialized = true;
                Ok(())
            }
            Err(error) => write_error(writer, id, error.into_error_object()),
        }
    }

    fn handle_approve<W>(
        &mut self,
        id: Value,
        params: Option<Value>,
        writer: &mut W,
    ) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        let params =
            match parse_params::<ApproveParams>(params, APPROVE_METHOD.qualified_name().as_str()) {
                Ok(params) => params,
                Err(error) => return write_error(writer, id, error),
            };

        match self.handler.approve(params) {
            Ok(output) => {
                write_json_line(writer, &JsonRpcResponse::new(id, output.result))?;
                emit_run_log_events(writer, &output.events)
            }
            Err(error) => write_error(writer, id, error.into_error_object()),
        }
    }

    fn handle_reject<W>(
        &mut self,
        id: Value,
        params: Option<Value>,
        writer: &mut W,
    ) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        let params =
            match parse_params::<RejectParams>(params, REJECT_METHOD.qualified_name().as_str()) {
                Ok(params) => params,
                Err(error) => return write_error(writer, id, error),
            };

        match self.handler.reject(params) {
            Ok(output) => {
                write_json_line(writer, &JsonRpcResponse::new(id, output.result))?;
                emit_run_log_events(writer, &output.events)
            }
            Err(error) => write_error(writer, id, error.into_error_object()),
        }
    }

    fn handle_send_turn<W>(
        &mut self,
        id: Value,
        params: Option<Value>,
        writer: &mut W,
    ) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        let params = match parse_params::<SendTurnParams>(
            params,
            SEND_TURN_METHOD.qualified_name().as_str(),
        ) {
            Ok(params) => params,
            Err(error) => return write_error(writer, id, error),
        };

        match self.handler.send_turn(params) {
            Ok(output) => {
                write_json_line(writer, &JsonRpcResponse::new(id, output.result))?;
                emit_run_log_events(writer, &output.events)
            }
            Err(error) => write_error(writer, id, error.into_error_object()),
        }
    }

    fn handle_resume<W>(
        &mut self,
        id: Value,
        params: Option<Value>,
        writer: &mut W,
    ) -> Result<(), AgentRpcError>
    where
        W: Write,
    {
        let params =
            match parse_params::<ResumeParams>(params, RESUME_METHOD.qualified_name().as_str()) {
                Ok(params) => params,
                Err(error) => return write_error(writer, id, error),
            };

        match self.handler.resume(params) {
            Ok(output) => {
                write_json_line(writer, &JsonRpcResponse::new(id, output.result))?;
                emit_run_log_events(writer, &output.events)
            }
            Err(error) => write_error(writer, id, error.into_error_object()),
        }
    }
}

pub fn run_stdio_request_loop<R, W, H>(
    reader: R,
    writer: &mut W,
    handler: H,
) -> Result<H, AgentRpcError>
where
    R: BufRead,
    W: Write,
    H: AgentRpcRequestHandler,
{
    let mut server = AgentRpcServer::new(handler);
    for line in reader.lines() {
        server.handle_line(&line?, writer)?;
    }
    Ok(server.into_inner())
}

pub fn write_json_line<W, T>(writer: &mut W, message: &T) -> Result<(), AgentRpcError>
where
    W: Write,
    T: Serialize,
{
    serde_json::to_writer(&mut *writer, message)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn write_error<W>(writer: &mut W, id: Value, error: JsonRpcErrorObject) -> Result<(), AgentRpcError>
where
    W: Write,
{
    write_json_line(writer, &JsonRpcErrorResponse::new(id, error))
}

fn emit_run_log_events<W>(writer: &mut W, events: &[RunLogEvent]) -> Result<(), AgentRpcError>
where
    W: Write,
{
    for event in events {
        let notification = run_log_event_to_notification(event)?;
        write_json_line(writer, &notification)?;
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
struct IncomingMessage {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

fn parse_incoming_message(line: &str) -> Result<IncomingMessage, AgentRpcParseError> {
    let value: Value = serde_json::from_str(line).map_err(|source| AgentRpcParseError {
        code: JSON_RPC_PARSE_ERROR,
        id: Value::Null,
        message: format!("JSON-RPC parse error: {source}"),
    })?;
    let object = value.as_object().ok_or_else(|| AgentRpcParseError {
        code: JSON_RPC_INVALID_REQUEST,
        id: Value::Null,
        message: "JSON-RPC message must be an object".to_owned(),
    })?;
    let error_id = object.get("id").cloned().unwrap_or(Value::Null);
    let jsonrpc = object
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentRpcParseError {
            code: JSON_RPC_INVALID_REQUEST,
            id: error_id.clone(),
            message: "JSON-RPC message is missing string field `jsonrpc`".to_owned(),
        })?
        .to_owned();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentRpcParseError {
            code: JSON_RPC_INVALID_REQUEST,
            id: error_id,
            message: "JSON-RPC message is missing string field `method`".to_owned(),
        })?
        .to_owned();
    let id = object.get("id").cloned();
    let params = object.get("params").cloned();

    Ok(IncomingMessage {
        jsonrpc,
        id,
        method,
        params,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct AgentRpcParseError {
    code: i64,
    id: Value,
    message: String,
}

impl AgentRpcParseError {
    fn into_error_object(self) -> JsonRpcErrorObject {
        JsonRpcErrorObject::new(self.code, self.message)
    }
}

fn parse_params<T>(params: Option<Value>, method: &str) -> Result<T, JsonRpcErrorObject>
where
    T: for<'de> Deserialize<'de>,
{
    let params = params.unwrap_or(Value::Null);
    serde_json::from_value(params).map_err(|source| {
        JsonRpcErrorObject::new(
            JSON_RPC_INVALID_PARAMS,
            format!("invalid params for {method}: {source}"),
        )
    })
}

#[derive(Debug, Error)]
pub enum AgentRpcError {
    #[error("JSON-RPC message serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("JSON-RPC stdio write failed: {0}")]
    Io(#[from] io::Error),
    #[error("event timestamp exceeds supported range: {time_unix_ms}")]
    TimestampOutOfRange { time_unix_ms: u64 },
}

fn format_unix_millis(time_unix_ms: u64) -> Result<String, AgentRpcError> {
    let seconds = time_unix_ms / 1_000;
    let millis = time_unix_ms % 1_000;
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_unix_days(days, time_unix_ms)?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

fn civil_from_unix_days(
    days_since_unix_epoch: u64,
    time_unix_ms: u64,
) -> Result<(i64, u32, u32), AgentRpcError> {
    let days_since_unix_epoch = i64::try_from(days_since_unix_epoch)
        .map_err(|_| AgentRpcError::TimestampOutOfRange { time_unix_ms })?;
    let z = days_since_unix_epoch
        .checked_add(719_468)
        .ok_or(AgentRpcError::TimestampOutOfRange { time_unix_ms })?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    Ok((
        year,
        u32::try_from(month).map_err(|_| AgentRpcError::TimestampOutOfRange { time_unix_ms })?,
        u32::try_from(day).map_err(|_| AgentRpcError::TimestampOutOfRange { time_unix_ms })?,
    ))
}

#[cfg(test)]
mod tests {
    use deepseek_coder_agent_core::{run_log::RunLogEvent, turn_loop::TurnEventSink};
    use serde_json::{Value, json};
    use std::io::Cursor;

    use super::{
        APPROVE_METHOD, AgentInitializeParams, AgentInitializeResult, AgentRpcHandlerError,
        AgentRpcHandlerOutput, AgentRpcRequestHandler, ApproveParams, ApproveResult, EVENT_METHOD,
        INITIALIZE_METHOD, JSON_RPC_INVALID_REQUEST, JSON_RPC_METHOD_NOT_FOUND,
        JSON_RPC_PARSE_ERROR, PROTOCOL_VERSION, REJECT_METHOD, RESUME_METHOD,
        RPC_UNSUPPORTED_PROTOCOL, RejectParams, RejectResult, ResumeParams, ResumeResult,
        RpcApprovalPersistence, RpcApprovalState, SEND_TURN_METHOD, SendTurnParams, SendTurnResult,
        StdioEventBridge, format_unix_millis, run_log_event_to_notification,
        run_stdio_request_loop,
    };

    #[test]
    fn method_names_match_protocol_docs() {
        assert_eq!(INITIALIZE_METHOD.qualified_name(), "agent.initialize");
        assert_eq!(SEND_TURN_METHOD.qualified_name(), "agent.sendTurn");
        assert_eq!(APPROVE_METHOD.qualified_name(), "agent.approve");
        assert_eq!(REJECT_METHOD.qualified_name(), "agent.reject");
        assert_eq!(CANCEL_METHOD.qualified_name(), "agent.cancel");
        assert_eq!(RESUME_METHOD.qualified_name(), "agent.resume");
        assert_eq!(EVENT_METHOD.qualified_name(), "agent.event");
    }

    #[test]
    fn run_log_event_converts_to_agent_event_notification() {
        let event = RunLogEvent {
            seq: 7,
            time_unix_ms: 123,
            event_type: "assistant.delta".to_owned(),
            run_id: "run_01".to_owned(),
            turn_id: Some("turn_01".to_owned()),
            payload: json!({ "text": "hello" }),
        };

        let notification =
            run_log_event_to_notification(&event).expect("notification should convert");
        assert_eq!(notification.jsonrpc, "2.0");
        assert_eq!(notification.method, "agent.event");
        assert_eq!(notification.params.seq, 7);
        assert_eq!(notification.params.time, "1970-01-01T00:00:00.123Z");
        assert_eq!(notification.params.event_type, "assistant.delta");
        assert_eq!(notification.params.run_id, "run_01");
        assert_eq!(notification.params.turn_id.as_deref(), Some("turn_01"));
        assert_eq!(notification.params.payload["text"], "hello");
    }

    #[test]
    fn stdio_bridge_writes_newline_delimited_notifications() {
        let events = vec![
            RunLogEvent {
                seq: 1,
                time_unix_ms: 0,
                event_type: "run.started".to_owned(),
                run_id: "run_01".to_owned(),
                turn_id: None,
                payload: json!({ "mode": "ask" }),
            },
            RunLogEvent {
                seq: 2,
                time_unix_ms: 86_400_000,
                event_type: "run.completed".to_owned(),
                run_id: "run_01".to_owned(),
                turn_id: Some("turn_01".to_owned()),
                payload: json!({ "summary": "done" }),
            },
        ];
        let mut bridge = StdioEventBridge::new(Vec::new());

        let envelopes = bridge
            .emit_events(&events)
            .expect("events should be emitted");

        assert_eq!(envelopes.len(), 2);
        let output = String::from_utf8(bridge.into_inner()).expect("stdio output must be UTF-8");
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).expect("first line should be JSON");
        let second: Value = serde_json::from_str(lines[1]).expect("second line should be JSON");
        assert_eq!(first["jsonrpc"], "2.0");
        assert_eq!(first["method"], "agent.event");
        assert_eq!(first["params"]["seq"], 1);
        assert_eq!(first["params"]["runId"], "run_01");
        assert!(first["params"].get("turnId").is_none());
        assert_eq!(second["params"]["time"], "1970-01-02T00:00:00.000Z");
        assert_eq!(second["params"]["payload"]["summary"], "done");
    }

    #[test]
    fn stdio_bridge_can_stream_turn_loop_events() {
        let event = RunLogEvent {
            seq: 1,
            time_unix_ms: 0,
            event_type: "assistant.delta".to_owned(),
            run_id: "run_01".to_owned(),
            turn_id: Some("turn_01".to_owned()),
            payload: json!({ "text": "hello", "stream": true }),
        };
        let mut bridge = StdioEventBridge::new(Vec::new());

        bridge
            .on_event(&event)
            .expect("turn event should be streamed");

        let output = String::from_utf8(bridge.into_inner()).expect("stdio output must be UTF-8");
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let value: Value = serde_json::from_str(lines[0]).expect("line should be JSON");
        assert_eq!(value["method"], EVENT_METHOD.qualified_name());
        assert_eq!(value["params"]["type"], "assistant.delta");
        assert_eq!(value["params"]["payload"]["text"], "hello");
    }

    #[test]
    fn unix_millis_formatter_handles_leap_day() {
        let formatted =
            format_unix_millis(951_782_400_000).expect("2000-02-29 timestamp should format");

        assert_eq!(formatted, "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn request_loop_initializes_and_accepts_turns() {
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": "init_1",
                "method": "agent.initialize",
                "params": initialize_params()
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "turn_1",
                "method": "agent.sendTurn",
                "params": {
                    "runId": "run_rpc",
                    "message": "Read README",
                    "mode": "ask"
                }
            })
            .to_string(),
        ]
        .join("\n");
        let mut output = Vec::new();

        let handler =
            run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
                .expect("request loop should complete");

        assert_eq!(handler.initialized.len(), 1);
        assert_eq!(handler.send_turns.len(), 1);
        assert_eq!(handler.send_turns[0].message, "Read README");
        let lines = output_lines(output);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["id"], "init_1");
        assert_eq!(lines[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(lines[1]["id"], "turn_1");
        assert_eq!(lines[1]["result"]["accepted"], true);
        assert_eq!(lines[1]["result"]["runId"], "run_rpc");
        assert_eq!(lines[2]["method"], "agent.event");
        assert_eq!(lines[2]["params"]["type"], "run.started");
    }

    #[test]
    fn request_loop_rejects_send_turn_before_initialize() {
        let input = json!({
            "jsonrpc": "2.0",
            "id": "turn_1",
            "method": "agent.sendTurn",
            "params": {
                "message": "hello",
                "mode": "ask"
            }
        })
        .to_string();
        let mut output = Vec::new();

        run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
            .expect("request loop should write an error response");

        let lines = output_lines(output);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["id"], "turn_1");
        assert_eq!(lines[0]["error"]["code"], JSON_RPC_INVALID_REQUEST);
        assert!(
            lines[0]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("agent.initialize"))
        );
    }

    #[test]
    fn request_loop_rejects_unsupported_protocol_without_initializing() {
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": "bad_init",
                "method": "agent.initialize",
                "params": {
                    "protocolVersion": "9.9.9",
                    "client": {
                        "name": "test-client",
                        "version": "0.1.0",
                        "frontend": "cli"
                    },
                    "workspaceRoot": "C:/workspace/project",
                    "workspaceTrusted": true
                }
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "good_init",
                "method": "agent.initialize",
                "params": initialize_params()
            })
            .to_string(),
        ]
        .join("\n");
        let mut output = Vec::new();

        let handler =
            run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
                .expect("request loop should complete");

        assert_eq!(handler.initialized.len(), 1);
        let lines = output_lines(output);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], RPC_UNSUPPORTED_PROTOCOL);
        assert_eq!(lines[1]["id"], "good_init");
        assert_eq!(lines[1]["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn request_loop_replays_resume_events_after_response() {
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": "init_1",
                "method": "agent.initialize",
                "params": initialize_params()
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "resume_1",
                "method": "agent.resume",
                "params": {
                    "runId": "run_rpc",
                    "replayFromSeq": 2
                }
            })
            .to_string(),
        ]
        .join("\n");
        let mut output = Vec::new();

        let handler =
            run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
                .expect("request loop should complete");

        assert_eq!(handler.resumes.len(), 1);
        assert_eq!(handler.resumes[0].replay_from_seq, Some(2));
        let lines = output_lines(output);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["id"], "resume_1");
        assert_eq!(lines[1]["result"]["nextSeq"], 3);
        assert_eq!(lines[2]["method"], "agent.event");
        assert_eq!(lines[2]["params"]["seq"], 2);
    }

    #[test]
    fn request_loop_dispatches_approval_decisions() {
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": "init_1",
                "method": "agent.initialize",
                "params": initialize_params()
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "approve_1",
                "method": "agent.approve",
                "params": {
                    "approvalId": "approval_1",
                    "persist": "session"
                }
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "reject_1",
                "method": "agent.reject",
                "params": {
                    "approvalId": "approval_2",
                    "reason": "not safe"
                }
            })
            .to_string(),
        ]
        .join("\n");
        let mut output = Vec::new();

        let handler =
            run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
                .expect("request loop should complete");

        assert_eq!(handler.approvals.len(), 1);
        assert_eq!(handler.approvals[0].approval_id, "approval_1");
        assert_eq!(
            handler.approvals[0].persist,
            Some(RpcApprovalPersistence::Session)
        );
        assert_eq!(handler.rejections.len(), 1);
        assert_eq!(handler.rejections[0].approval_id, "approval_2");
        let lines = output_lines(output);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["id"], "approve_1");
        assert_eq!(lines[1]["result"]["state"], "approved");
        assert_eq!(lines[1]["result"]["persist"], "session");
        assert_eq!(lines[2]["id"], "reject_1");
        assert_eq!(lines[2]["result"]["state"], "rejected");
        assert_eq!(lines[2]["result"]["reason"], "not safe");
    }

    #[test]
    fn request_loop_writes_parse_and_method_errors() {
        let input = [
            "{not json}".to_owned(),
            json!({
                "jsonrpc": "2.0",
                "id": "init_1",
                "method": "agent.initialize",
                "params": initialize_params()
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": "missing_1",
                "method": "agent.missing",
                "params": {}
            })
            .to_string(),
        ]
        .join("\n");
        let mut output = Vec::new();

        run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
            .expect("request loop should complete after errors");

        let lines = output_lines(output);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["id"], Value::Null);
        assert_eq!(lines[0]["error"]["code"], JSON_RPC_PARSE_ERROR);
        assert_eq!(lines[2]["id"], "missing_1");
        assert_eq!(lines[2]["error"]["code"], JSON_RPC_METHOD_NOT_FOUND);
    }

    #[test]
    fn request_loop_preserves_id_for_invalid_request_errors() {
        let input = json!({
            "jsonrpc": "2.0",
            "id": "bad_request",
            "params": {}
        })
        .to_string();
        let mut output = Vec::new();

        run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
            .expect("request loop should write an invalid request error");

        let lines = output_lines(output);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["id"], "bad_request");
        assert_eq!(lines[0]["error"]["code"], JSON_RPC_INVALID_REQUEST);
    }

    #[test]
    fn request_loop_does_not_respond_to_notifications() {
        let input = json!({
            "jsonrpc": "2.0",
            "method": "agent.initialize",
            "params": initialize_params()
        })
        .to_string();
        let mut output = Vec::new();

        let handler =
            run_stdio_request_loop(Cursor::new(input), &mut output, TestHandler::default())
                .expect("request loop should ignore notifications");

        assert!(handler.initialized.is_empty());
        assert!(output.is_empty());
    }

    fn initialize_params() -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "client": {
                "name": "test-client",
                "version": "0.1.0",
                "frontend": "cli"
            },
            "workspaceRoot": "C:/workspace/project",
            "workspaceTrusted": true
        })
    }

    fn output_lines(output: Vec<u8>) -> Vec<Value> {
        String::from_utf8(output)
            .expect("output should be UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("line should be JSON"))
            .collect()
    }

    #[derive(Default)]
    struct TestHandler {
        initialized: Vec<AgentInitializeParams>,
        send_turns: Vec<SendTurnParams>,
        approvals: Vec<ApproveParams>,
        rejections: Vec<RejectParams>,
        resumes: Vec<ResumeParams>,
    }

    impl AgentRpcRequestHandler for TestHandler {
        fn initialize(
            &mut self,
            params: AgentInitializeParams,
        ) -> Result<AgentInitializeResult, AgentRpcHandlerError> {
            self.initialized.push(params);
            Ok(AgentInitializeResult::default())
        }

        fn send_turn(
            &mut self,
            params: SendTurnParams,
        ) -> Result<AgentRpcHandlerOutput<SendTurnResult>, AgentRpcHandlerError> {
            let run_id = params
                .run_id
                .clone()
                .unwrap_or_else(|| "run_rpc".to_owned());
            self.send_turns.push(params);
            Ok(AgentRpcHandlerOutput::new(SendTurnResult {
                run_id: run_id.clone(),
                turn_id: "turn_rpc".to_owned(),
                accepted: true,
            })
            .with_events(vec![run_log_event(
                1,
                "run.started",
                &run_id,
                None,
                json!({ "mode": "ask" }),
            )]))
        }

        fn approve(
            &mut self,
            params: ApproveParams,
        ) -> Result<AgentRpcHandlerOutput<ApproveResult>, AgentRpcHandlerError> {
            self.approvals.push(params.clone());
            Ok(AgentRpcHandlerOutput::new(ApproveResult {
                approval_id: params.approval_id,
                state: RpcApprovalState::Approved,
                persist: params.persist.unwrap_or(RpcApprovalPersistence::Never),
            }))
        }

        fn reject(
            &mut self,
            params: RejectParams,
        ) -> Result<AgentRpcHandlerOutput<RejectResult>, AgentRpcHandlerError> {
            self.rejections.push(params.clone());
            Ok(AgentRpcHandlerOutput::new(RejectResult {
                approval_id: params.approval_id,
                state: RpcApprovalState::Rejected,
                reason: params.reason,
            }))
        }

        fn resume(
            &mut self,
            params: ResumeParams,
        ) -> Result<AgentRpcHandlerOutput<ResumeResult>, AgentRpcHandlerError> {
            let run_id = params.run_id.clone();
            self.resumes.push(params);
            Ok(AgentRpcHandlerOutput::new(ResumeResult {
                run_id: run_id.clone(),
                next_seq: 3,
                replay_started: true,
            })
            .with_events(vec![run_log_event(
                2,
                "assistant.delta",
                &run_id,
                Some("turn_rpc"),
                json!({ "text": "hello", "stream": true }),
            )]))
        }
    }

    fn run_log_event(
        seq: u64,
        event_type: &str,
        run_id: &str,
        turn_id: Option<&str>,
        payload: Value,
    ) -> RunLogEvent {
        RunLogEvent {
            seq,
            time_unix_ms: 0,
            event_type: event_type.to_owned(),
            run_id: run_id.to_owned(),
            turn_id: turn_id.map(str::to_owned),
            payload,
        }
    }
}
