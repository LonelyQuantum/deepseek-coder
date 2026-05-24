#![forbid(unsafe_code)]

use std::{fs, process::Command};

use deepseek_coder_agent_core::run_log::RunLogStore;
use deepseek_coder_agent_rpc::JSON_RPC_INVALID_PARAMS;
use serde_json::Value;

#[test]
fn fixture_readme_json_smoke_from_binary() {
    let workspace = TestWorkspace::new();
    workspace.write("README.md", "hello from process smoke\n");

    let output = Command::new(env!("CARGO_BIN_EXE_deepseek-coder"))
        .args([
            "run",
            "--provider",
            "fixture",
            "--fixture",
            "readme",
            "--json",
            "--workspace",
            workspace.path_str(),
            "--run-id",
            "run_cli_process_smoke",
            "--turn-id",
            "turn_cli_process_smoke",
            "Read README",
        ])
        .output()
        .expect("CLI binary should run");

    assert!(
        output.status.success(),
        "CLI failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr should be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let notifications = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("stdout line should be JSON"))
        .collect::<Vec<_>>();

    assert!(!notifications.is_empty());
    assert!(
        notifications
            .iter()
            .all(|value| value["method"] == "agent.event")
    );

    for (index, notification) in notifications.iter().enumerate() {
        assert_eq!(
            notification["params"]["seq"].as_u64(),
            Some((index + 1) as u64)
        );
    }
    let event_types = notifications
        .iter()
        .map(|value| {
            value["params"]["type"]
                .as_str()
                .expect("event type should be a string")
        })
        .collect::<Vec<_>>();
    assert_event_subsequence(
        &event_types,
        &[
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
        ],
    );

    let store = RunLogStore::new(workspace.path()).expect("run log store should open");
    let events = store
        .load_run("run_cli_process_smoke")
        .expect("run log should load");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "run.completed")
    );
}

#[test]
fn run_json_usage_error_from_binary_is_json_rpc_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_deepseek-coder"))
        .args(["run", "--json", "--provider", "fixture"])
        .output()
        .expect("CLI binary should run");

    assert!(!output.status.success());
    assert!(
        output.stderr.is_empty(),
        "stderr should be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let lines = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("stdout line should be JSON"))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["jsonrpc"], "2.0");
    assert_eq!(lines[0]["id"], "cli.run");
    assert_eq!(lines[0]["error"]["code"], JSON_RPC_INVALID_PARAMS);
    assert_eq!(
        lines[0]["error"]["data"]["symbolicCode"],
        "E_INVALID_PARAMS"
    );
    assert_eq!(lines[0]["error"]["data"]["kind"], "usage");
}

fn assert_event_subsequence(actual: &[&str], expected: &[&str]) {
    let mut search_from = 0;
    for expected_type in expected {
        let offset = actual[search_from..]
            .iter()
            .position(|actual_type| actual_type == expected_type)
            .unwrap_or_else(|| {
                panic!(
                    "missing event `{expected_type}` after position {search_from}; actual events: {actual:?}"
                )
            });
        search_from += offset + 1;
    }
}

struct TestWorkspace {
    path: std::path::PathBuf,
    path_string: String,
}

impl TestWorkspace {
    fn new() -> Self {
        let unique = format!(
            "deepseek-coder-cli-process-test-{}-{}",
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
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
