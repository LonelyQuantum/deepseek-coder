#![forbid(unsafe_code)]

use std::io::{self, Write};

use deepseek_coder_agent_core::run_log::RunLogEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    use deepseek_coder_agent_core::run_log::RunLogEvent;
    use serde_json::{Value, json};

    use super::{
        EVENT_METHOD, INITIALIZE_METHOD, StdioEventBridge, format_unix_millis,
        run_log_event_to_notification,
    };

    #[test]
    fn method_names_match_protocol_docs() {
        assert_eq!(INITIALIZE_METHOD.qualified_name(), "agent.initialize");
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
    fn unix_millis_formatter_handles_leap_day() {
        let formatted =
            format_unix_millis(951_782_400_000).expect("2000-02-29 timestamp should format");

        assert_eq!(formatted, "2000-02-29T00:00:00.000Z");
    }
}
