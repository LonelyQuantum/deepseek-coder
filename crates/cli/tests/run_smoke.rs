#![forbid(unsafe_code)]

use std::{
    io::Write,
    process::{Command, Stdio},
};

use prole_coder_agent_core::{run_log::RunLogStore, test_helpers::TestWorkspace};
use prole_coder_agent_rpc::{JSON_RPC_INVALID_PARAMS, PROTOCOL_VERSION};
use serde_json::{Value, json};

#[test]
fn fixture_readme_json_smoke_from_binary() {
    let workspace = TestWorkspace::new("cli-process");
    workspace.write("README.md", "hello from process smoke\n");

    let output = Command::new(env!("CARGO_BIN_EXE_prole"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_prole"))
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

#[test]
fn rpc_fixture_smoke_from_binary() {
    let workspace = TestWorkspace::new("cli-rpc-process");
    workspace.write("README.md", "hello from rpc process smoke\n");
    let input = [
        json!({
            "jsonrpc": "2.0",
            "id": "init_1",
            "method": "agent.initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "client": {
                    "name": "cli-process-test",
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
                "runId": "run_cli_rpc_process_smoke",
                "message": "Read README",
                "mode": "ask"
            }
        })
        .to_string(),
    ]
    .join("\n");
    let mut child = Command::new(env!("CARGO_BIN_EXE_prole"))
        .args(["rpc", "--provider", "fixture", "--fixture", "readme"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CLI binary should spawn");

    {
        let stdin = child.stdin.as_mut().expect("child stdin should be piped");
        stdin
            .write_all(input.as_bytes())
            .expect("input should be written");
        stdin.write_all(b"\n").expect("newline should be written");
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .expect("CLI binary should finish after stdin EOF");

    assert!(
        output.status.success(),
        "CLI rpc failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

    assert!(lines.iter().any(|line| {
        line["id"] == "init_1" && line["result"]["protocolVersion"] == PROTOCOL_VERSION
    }));
    assert!(lines.iter().any(|line| {
        line["id"] == "turn_1"
            && line["result"]["accepted"] == true
            && line["result"]["runId"] == "run_cli_rpc_process_smoke"
    }));

    let notifications = lines
        .iter()
        .filter(|line| line["method"] == "agent.event")
        .collect::<Vec<_>>();
    assert!(!notifications.is_empty());
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
        .load_run("run_cli_rpc_process_smoke")
        .expect("run log should load");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "run.completed")
    );
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
