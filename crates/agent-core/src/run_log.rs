use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::DEFAULT_STATE_DIR;

const RUNS_DIR: &str = "runs";
const EVENTS_FILE: &str = "events.jsonl";

pub const REDACTED_VALUE: &str = "<redacted>";

#[derive(Debug, Clone)]
pub struct RunLogStore {
    workspace_root: PathBuf,
    state_dir: PathBuf,
    runs_dir: PathBuf,
}

impl RunLogStore {
    pub fn new(workspace_root: impl AsRef<Path>) -> Result<Self, RunLogError> {
        Self::with_state_dir(workspace_root, DEFAULT_STATE_DIR)
    }

    pub fn with_state_dir(
        workspace_root: impl AsRef<Path>,
        state_dir: impl AsRef<Path>,
    ) -> Result<Self, RunLogError> {
        let workspace_root =
            fs::canonicalize(workspace_root.as_ref()).map_err(|source| RunLogError::Io {
                path: workspace_root.as_ref().to_path_buf(),
                source,
            })?;
        if !workspace_root.is_dir() {
            return Err(RunLogError::WorkspaceRootNotDirectory {
                path: workspace_root,
            });
        }

        let state_dir = normalize_workspace_relative_path(state_dir.as_ref())?;
        let state_dir = workspace_root.join(state_dir);
        let runs_dir = state_dir.join(RUNS_DIR);

        Ok(Self {
            workspace_root,
            state_dir,
            runs_dir,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn runs_dir(&self) -> &Path {
        &self.runs_dir
    }

    pub fn create_run(&self, run_id: impl Into<String>) -> Result<RunLog, RunLogError> {
        let run_id = validate_id("run id", run_id.into())?;
        let run_dir = self.run_dir(&run_id)?;
        if run_dir.exists() {
            return Err(RunLogError::RunAlreadyExists { run_id });
        }

        fs::create_dir_all(&run_dir).map_err(|source| RunLogError::Io {
            path: run_dir.clone(),
            source,
        })?;
        let events_path = run_dir.join(EVENTS_FILE);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&events_path)
            .map_err(|source| RunLogError::Io {
                path: events_path.clone(),
                source,
            })?;

        Ok(RunLog::new(run_id, events_path, 1))
    }

    pub fn open_run(&self, run_id: impl Into<String>) -> Result<RunLog, RunLogError> {
        let run_id = validate_id("run id", run_id.into())?;
        let events_path = self.events_path(&run_id)?;
        if !events_path.is_file() {
            return Err(RunLogError::RunNotFound { run_id });
        }

        let next_seq = next_sequence_from_events(&run_id, &events_path)?;
        Ok(RunLog::new(run_id, events_path, next_seq))
    }

    pub fn open_or_create_run(&self, run_id: impl Into<String>) -> Result<RunLog, RunLogError> {
        let run_id = validate_id("run id", run_id.into())?;
        let events_path = self.events_path(&run_id)?;
        if events_path.is_file() {
            return self.open_run(run_id);
        }

        self.create_run(run_id)
    }

    pub fn load_run(&self, run_id: impl Into<String>) -> Result<Vec<RunLogEvent>, RunLogError> {
        let run_id = validate_id("run id", run_id.into())?;
        let events_path = self.events_path(&run_id)?;
        if !events_path.is_file() {
            return Err(RunLogError::RunNotFound { run_id });
        }

        read_events(&run_id, &events_path)
    }

    pub fn events_path(&self, run_id: &str) -> Result<PathBuf, RunLogError> {
        let run_id = validate_id("run id", run_id.to_owned())?;
        Ok(self.run_dir(&run_id)?.join(EVENTS_FILE))
    }

    fn run_dir(&self, run_id: &str) -> Result<PathBuf, RunLogError> {
        let run_id = validate_id("run id", run_id.to_owned())?;
        Ok(self.runs_dir.join(run_id))
    }
}

#[derive(Debug)]
pub struct RunLog {
    run_id: String,
    events_path: PathBuf,
    next_seq: u64,
}

impl RunLog {
    fn new(run_id: String, events_path: PathBuf, next_seq: u64) -> Self {
        Self {
            run_id,
            events_path,
            next_seq,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn events_path(&self) -> &Path {
        &self.events_path
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn append(
        &mut self,
        event_type: impl Into<String>,
        turn_id: Option<String>,
        payload: Value,
    ) -> Result<RunLogEvent, RunLogError> {
        self.append_at(unix_time_millis()?, event_type, turn_id, payload)
    }

    fn append_at(
        &mut self,
        time_unix_ms: u64,
        event_type: impl Into<String>,
        turn_id: Option<String>,
        payload: Value,
    ) -> Result<RunLogEvent, RunLogError> {
        let event_type = validate_event_type(event_type.into())?;
        let turn_id = turn_id.map(|id| validate_id("turn id", id)).transpose()?;
        let event = RunLogEvent {
            seq: self.next_seq,
            time_unix_ms,
            event_type,
            run_id: self.run_id.clone(),
            turn_id,
            payload: redact_value(payload),
        };

        append_event(&self.events_path, &event)?;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(RunLogError::SequenceOverflow)?;
        Ok(event)
    }

    pub fn load(&self) -> Result<Vec<RunLogEvent>, RunLogError> {
        read_events(&self.run_id, &self.events_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEvent {
    pub seq: u64,
    pub time_unix_ms: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Error)]
pub enum RunLogError {
    #[error("workspace root is not a directory: {path}")]
    WorkspaceRootNotDirectory { path: PathBuf },
    #[error("run already exists: {run_id}")]
    RunAlreadyExists { run_id: String },
    #[error("run log not found: {run_id}")]
    RunNotFound { run_id: String },
    #[error("invalid {kind}: {value}")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("invalid event type: {value}")]
    InvalidEventType { value: String },
    #[error("path must be workspace-relative: {path}")]
    InvalidStatePath { path: PathBuf },
    #[error("run log sequence overflow")]
    SequenceOverflow,
    #[error("run log sequence mismatch in {path}: expected {expected}, got {actual}")]
    SequenceMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("run id mismatch in {path}: expected `{expected}`, got `{actual}`")]
    RunIdMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("invalid run log JSON in {path} at line {line}: {source}")]
    InvalidJson {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    #[error("system clock is before UNIX epoch")]
    SystemClockBeforeUnixEpoch,
    #[error("I/O error for {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("serialization failed for {path}: {source}")]
    Serialization {
        path: PathBuf,
        source: serde_json::Error,
    },
}

fn append_event(path: &Path, event: &RunLogEvent) -> Result<(), RunLogError> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(false)
        .open(path)
        .map_err(|source| RunLogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    serde_json::to_writer(&mut file, event).map_err(|source| RunLogError::Serialization {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(b"\n").map_err(|source| RunLogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.flush().map_err(|source| RunLogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn next_sequence_from_events(run_id: &str, path: &Path) -> Result<u64, RunLogError> {
    let events = read_events(run_id, path)?;
    u64::try_from(events.len())
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or(RunLogError::SequenceOverflow)
}

fn read_events(run_id: &str, path: &Path) -> Result<Vec<RunLogEvent>, RunLogError> {
    let file = File::open(path).map_err(|source| RunLogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut events = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|source| RunLogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let event: RunLogEvent =
            serde_json::from_str(&line).map_err(|source| RunLogError::InvalidJson {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })?;
        let expected = u64::try_from(events.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(RunLogError::SequenceOverflow)?;
        if event.seq != expected {
            return Err(RunLogError::SequenceMismatch {
                path: path.to_path_buf(),
                expected,
                actual: event.seq,
            });
        }
        if event.run_id != run_id {
            return Err(RunLogError::RunIdMismatch {
                path: path.to_path_buf(),
                expected: run_id.to_owned(),
                actual: event.run_id,
            });
        }

        events.push(event);
    }

    Ok(events)
}

fn unix_time_millis() -> Result<u64, RunLogError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RunLogError::SystemClockBeforeUnixEpoch)?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn validate_id(kind: &'static str, value: String) -> Result<String, RunLogError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(RunLogError::InvalidIdentifier { kind, value });
    }

    Ok(value)
}

fn validate_event_type(value: String) -> Result<String, RunLogError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
    {
        return Err(RunLogError::InvalidEventType { value });
    }

    Ok(value)
}

fn normalize_workspace_relative_path(path: &Path) -> Result<PathBuf, RunLogError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(RunLogError::InvalidStatePath {
                    path: path.to_path_buf(),
                });
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(RunLogError::InvalidStatePath {
            path: path.to_path_buf(),
        });
    }

    Ok(normalized)
}

pub fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_object(map)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::String(text) => Value::String(redact_sensitive_string(&text)),
        other => other,
    }
}

fn redact_object(map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter()
        .map(|(key, value)| {
            if is_sensitive_key(&key) {
                (key, Value::String(REDACTED_VALUE.to_owned()))
            } else {
                (key, redact_value(value))
            }
        })
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey"
            | "deepseekapikey"
            | "authorization"
            | "password"
            | "secret"
            | "secretkey"
            | "token"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "credential"
            | "credentials"
            | "privatekey"
    )
}

fn redact_sensitive_string(text: &str) -> String {
    redact_secret_like_tokens(text)
}

fn redact_secret_like_tokens(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(relative_start) = text[index..].find("sk-") {
        let start = index + relative_start;
        output.push_str(&text[index..start]);
        let mut end = start + 3;
        for ch in text[end..].chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                end += ch.len_utf8();
            } else {
                break;
            }
        }

        if end - start >= 12 {
            output.push_str(REDACTED_VALUE);
        } else {
            output.push_str(&text[start..end]);
        }
        index = end;
    }
    output.push_str(&text[index..]);
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{REDACTED_VALUE, RunLogError, RunLogStore};

    #[test]
    fn run_log_appends_and_loads_events_with_monotonic_sequences() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("store should open");
        let mut run = store.create_run("run_test").expect("run should be created");

        let started = run
            .append_at(
                10,
                "run.started",
                None,
                json!({ "mode": "edit", "workspaceRoot": "workspace" }),
            )
            .expect("event should append");
        let delta = run
            .append_at(
                20,
                "assistant.delta",
                Some("turn_1".to_owned()),
                json!({ "text": "hello" }),
            )
            .expect("event should append");

        assert_eq!(started.seq, 1);
        assert_eq!(delta.seq, 2);
        assert_eq!(run.next_seq(), 3);

        let loaded = store.load_run("run_test").expect("events should load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].event_type, "run.started");
        assert_eq!(loaded[1].turn_id.as_deref(), Some("turn_1"));
        assert_eq!(loaded[1].payload["text"], "hello");

        let reopened = store.open_run("run_test").expect("run should reopen");
        assert_eq!(reopened.next_seq(), 3);
    }

    #[test]
    fn run_log_redacts_sensitive_payload_fields_and_secret_like_strings() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("store should open");
        let mut run = store
            .create_run("run_redaction")
            .expect("run should be created");
        let api_key = format!("sk-{}", "this-value-should-not-appear");
        let inline_secret = format!("visible sk-{}", "this-inline-secret-123");

        run.append_at(
            10,
            "tool.completed",
            Some("turn_1".to_owned()),
            json!({
                "apiKey": api_key,
                "authorization": "Bearer secret",
                "cacheHitTokens": 42,
                "stdout": inline_secret,
                "nested": {
                    "refresh_token": "secret-token"
                }
            }),
        )
        .expect("event should append");

        let loaded = store.load_run("run_redaction").expect("events should load");
        let payload = &loaded[0].payload;
        assert_eq!(payload["apiKey"], REDACTED_VALUE);
        assert_eq!(payload["authorization"], REDACTED_VALUE);
        assert_eq!(payload["cacheHitTokens"], 42);
        assert_eq!(payload["nested"]["refresh_token"], REDACTED_VALUE);
        assert_eq!(payload["stdout"], format!("visible {REDACTED_VALUE}"));
    }

    #[test]
    fn run_log_rejects_unsafe_run_ids_and_state_dirs() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("store should open");

        assert!(matches!(
            store.create_run("../outside"),
            Err(RunLogError::InvalidIdentifier { .. })
        ));
        assert!(matches!(
            RunLogStore::with_state_dir(workspace.path(), "../outside"),
            Err(RunLogError::InvalidStatePath { .. })
        ));
    }

    #[test]
    fn run_log_rejects_sequence_gaps() {
        let workspace = TestWorkspace::new();
        let store = RunLogStore::new(workspace.path()).expect("store should open");
        let mut run = store
            .create_run("run_corrupt")
            .expect("run should be created");
        run.append_at(10, "run.started", None, json!({}))
            .expect("event should append");

        fs::write(
            run.events_path(),
            concat!(
                r#"{"seq":1,"timeUnixMs":10,"type":"run.started","runId":"run_corrupt","payload":{}}"#,
                "\n",
                r#"{"seq":3,"timeUnixMs":20,"type":"run.completed","runId":"run_corrupt","payload":{}}"#,
                "\n"
            ),
        )
        .expect("corrupt log should be written");

        assert!(matches!(
            store.load_run("run_corrupt"),
            Err(RunLogError::SequenceMismatch { .. })
        ));
    }

    struct TestWorkspace {
        path: std::path::PathBuf,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let unique = format!(
                "deepseek-coder-run-log-test-{}-{}",
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
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
