#![forbid(unsafe_code)]

// Result display tests. These tests are intentionally separate from normal
// development tests: they print a readable agent transcript for humans while
// leaving protocol-level streaming events unchanged.

use std::{
    env,
    error::Error,
    fs,
    process::{Command, Output},
};

use deepseek_coder_agent_core::{
    provider::deepseek_api::{DEFAULT_API_BASE_URL, DEFAULT_MODEL},
    test_helpers::{LIVE_TEST_FLAG, TestWorkspace, live_api_key, repo_root_from_crate_manifest},
};
use serde_json::Value;

#[test]
#[ignore = "result display test; run `cargo demo`"]
fn fixture_agent_interaction_transcript_demo() -> Result<(), Box<dyn Error>> {
    let workspace = TestWorkspace::with_preserve("fixture-agent-demo");
    workspace.write("CLI_SMOKE.txt", "old\n");

    let verify_command = verification_command();
    let output = Command::new(env!("CARGO_BIN_EXE_deepseek-coder"))
        .args([
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
            "demo_fixture_agent",
            "--turn-id",
            "demo_fixture_agent_turn",
            "--verify",
            verify_command.as_str(),
            "--",
            "Patch smoke file and report the result.",
        ])
        .output()?;

    let demo_output = successful_agent_events(output, &workspace, "demo_fixture_agent")?;
    print_agent_transcript(
        "Fixture Agent Interaction Demo",
        &workspace,
        "demo_fixture_agent",
        &demo_output.notifications,
        &["CLI_SMOKE.txt"],
    );

    assert_event(&demo_output.notifications, "tool.completed");
    assert_event(&demo_output.notifications, "verification.completed");
    assert_event(&demo_output.notifications, "run.completed");
    assert_eq!(workspace.read("CLI_SMOKE.txt"), "new\n");

    Ok(())
}

#[test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, API key, network access, and local cargo"]
fn live_deepseek_agent_interaction_transcript_demo() -> Result<(), Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek agent demo: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(());
    }

    let api_key = live_api_key(repo_root_from_crate_manifest(env!("CARGO_MANIFEST_DIR")))?;
    let workspace = TestWorkspace::with_preserve("live-agent-demo");
    workspace.write(
        "Cargo.toml",
        r#"[package]
name = "deepseek-coder-agent-demo"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    );
    workspace.write(
        "README.md",
        "Small repository used to demonstrate a real deepseek-coder agent turn.\n",
    );
    workspace.write(
        "src/lib.rs",
        r#"pub fn greeting() -> &'static str {
    "old"
}

#[cfg(test)]
mod tests {
    #[test]
    fn greeting_is_updated() {
        assert_eq!(super::greeting(), "hello from deepseek-coder demo");
    }
}
"#,
    );
    workspace.git_init();
    workspace.git_commit_all("initial");

    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("DEEPSEEK_AGENT_DEMO_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let output = Command::new(env!("CARGO_BIN_EXE_deepseek-coder"))
        .env("DEEPSEEK_API_KEY", &api_key)
        .env("DEEPSEEK_BASE_URL", base_url)
        .env("DEEPSEEK_MODEL", model)
        .args([
            "run",
            "--provider",
            "deepseek",
            "--thinking",
            "disabled",
            "--mode",
            "edit",
            "--auto-approve",
            "--json",
            "--workspace",
            workspace.path_str(),
            "--run-id",
            "demo_live_agent",
            "--turn-id",
            "demo_live_agent_turn",
            "--max-model-turns",
            "6",
            "--max-output-tokens",
            "768",
            "--verify",
            "cargo test --quiet",
            "--verify-timeout-ms",
            "120000",
            "--",
            concat!(
                "This is a live deepseek-coder agent interaction demo. ",
                "You MUST use tools before answering. First call read_file for README.md and src/lib.rs. ",
                "Then use apply_patch to edit only src/lib.rs so greeting() returns exactly ",
                "hello from deepseek-coder demo. When calling apply_patch, expectedFiles MUST be a JSON array ",
                "exactly like [\"src/lib.rs\"], not a quoted string. Do not call shell; the harness will run cargo test. ",
                "After the patch is applied, reply with a short final answer containing OK_AGENT_DEMO."
            ),
        ])
        .output()?;

    let demo_output = successful_agent_events(output, &workspace, "demo_live_agent")?;
    assert!(!demo_output.stdout.contains(&api_key));
    assert!(!demo_output.stderr.contains(&api_key));
    print_agent_transcript(
        "Live DeepSeek Agent Interaction Demo",
        &workspace,
        "demo_live_agent",
        &demo_output.notifications,
        &["src/lib.rs"],
    );

    assert_event(&demo_output.notifications, "tool.completed");
    assert_event(&demo_output.notifications, "verification.completed");
    assert_event(&demo_output.notifications, "run.completed");
    assert!(
        workspace
            .read("src/lib.rs")
            .contains("hello from deepseek-coder demo")
    );

    Ok(())
}

struct AgentDemoOutput {
    notifications: Vec<Value>,
    stdout: String,
    stderr: String,
}

fn successful_agent_events(
    output: Output,
    workspace: &TestWorkspace,
    run_id: &str,
) -> Result<AgentDemoOutput, Box<dyn Error>> {
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    let (notifications, error_response) = split_agent_stdout(&stdout)?;

    if !output.status.success() {
        print_agent_transcript(
            "Failed Agent Interaction Transcript",
            workspace,
            run_id,
            &notifications,
            &[],
        );
        let error_message = error_response
            .as_ref()
            .and_then(|value| value["error"]["message"].as_str())
            .unwrap_or("CLI exited without a JSON-RPC error response");
        return Err(format!(
            "agent demo failed with exit code {:?}: {error_message}\nstderr: {}",
            output.status.code(),
            stderr.trim()
        )
        .into());
    }

    if let Some(error_response) = error_response {
        return Err(format!(
            "agent demo succeeded but emitted JSON-RPC error response: {}",
            error_response
        )
        .into());
    }

    Ok(AgentDemoOutput {
        notifications,
        stdout,
        stderr,
    })
}

fn split_agent_stdout(output: &str) -> Result<(Vec<Value>, Option<Value>), Box<dyn Error>> {
    let mut notifications = Vec::new();
    let mut error_response = None;

    for line in output.lines() {
        let value = serde_json::from_str::<Value>(line)?;
        if value["method"] == "agent.event" {
            notifications.push(value);
        } else if value["jsonrpc"] == "2.0" && value.get("error").is_some() {
            error_response = Some(value);
        } else {
            return Err(format!("unexpected JSON-RPC output line: {value}").into());
        }
    }

    Ok((notifications, error_response))
}

fn assert_event(notifications: &[Value], event_type: &str) {
    assert!(
        notifications
            .iter()
            .any(|value| value["params"]["type"] == event_type),
        "missing event type {event_type}"
    );
}

fn print_agent_transcript(
    title: &str,
    workspace: &TestWorkspace,
    run_id: &str,
    notifications: &[Value],
    files_to_show: &[&str],
) {
    println!();
    println!("=== {title} ===");
    println!("workspace: {}", workspace.path_str());
    println!(
        "workspace cleanup: {}",
        if workspace.is_preserved() {
            "preserved"
        } else {
            "temporary; set DEEPSEEK_CODER_KEEP_DEMO_WORKSPACE=1 to keep it"
        }
    );
    println!();
    println!("--- Agent events ---");
    print_agent_events(notifications);

    println!();
    println!("--- Final files ---");
    for relative in files_to_show {
        println!(">>> {relative}");
        println!("{}", workspace.read(relative).trim_end());
    }

    let summary_path = workspace
        .path()
        .join(".deepseek-coder")
        .join("runs")
        .join(run_id)
        .join("summary.json");
    if let Ok(summary) = fs::read_to_string(summary_path) {
        println!();
        println!("--- Run summary ---");
        println!("{}", summary.trim_end());
    }
}

fn print_agent_events(notifications: &[Value]) {
    let mut index = 0;
    while index < notifications.len() {
        if event_type(&notifications[index]) == Some("assistant.delta") {
            let start_seq = event_seq(&notifications[index]);
            let iteration = assistant_iteration(&notifications[index]);
            let mut end_seq = start_seq;
            let mut text = String::new();

            while index < notifications.len()
                && event_type(&notifications[index]) == Some("assistant.delta")
                && assistant_iteration(&notifications[index]) == iteration
            {
                end_seq = event_seq(&notifications[index]);
                text.push_str(assistant_delta_text(&notifications[index]));
                index += 1;
            }

            let range = match (start_seq, end_seq) {
                (Some(start), Some(end)) if start != end => format!("{start:03}-{end:03}"),
                (Some(seq), _) => format!("{seq:03}"),
                _ => "---".to_owned(),
            };
            let iteration = iteration
                .map(|value| format!(" iteration={value}"))
                .unwrap_or_default();
            println!(
                "{range} assistant.delta{iteration}: text={}",
                truncate(&text, 320)
            );
            continue;
        }

        println!("{}", event_summary(&notifications[index]));
        index += 1;
    }
}

fn event_summary(notification: &Value) -> String {
    let params = &notification["params"];
    let seq = params["seq"]
        .as_u64()
        .map(|seq| format!("{seq:03}"))
        .unwrap_or_else(|| "---".to_owned());
    let event_type = params["type"].as_str().unwrap_or("<unknown>");
    let payload = &params["payload"];
    let detail = match event_type {
        "run.started" => format!(
            "mode={} workspace={}",
            field(payload, "mode"),
            field(payload, "workspaceRoot")
        ),
        "context.built" => format!(
            "tokens={} sources={}",
            field(payload, "inputTokens"),
            array_len_field(payload, "includedSources")
        ),
        "provider.requested" => format!(
            "iteration={} messages={} replay={}",
            field(payload, "iteration"),
            field(payload, "messageCount"),
            field(payload, "reasoningState")
        ),
        "tool.requested" | "tool.started" => {
            format!(
                "name={} callId={}",
                field(payload, "name"),
                field(payload, "toolCallId")
            )
        }
        "tool.approvalRequired" => {
            format!(
                "name={} risk={}",
                field(payload, "toolName"),
                field(payload, "risk")
            )
        }
        "tool.approvalResolved" => format!("decision={}", field(payload, "decision")),
        "tool.completed" => format!(
            "name={} status={} changedFiles={}",
            field(payload, "name"),
            field(payload, "status"),
            nested_field(payload, "result", "files")
        ),
        "verification.started" => format!("command={}", field(payload, "command")),
        "verification.completed" => format!(
            "status={} exitCode={} stdout={}",
            field(payload, "status"),
            field(payload, "exitCode"),
            field(payload, "stdout")
        ),
        "run.completed" => format!(
            "changedFiles={} summary={}",
            field(payload, "changedFiles"),
            field(payload, "summary")
        ),
        "run.failed" => format!(
            "code={} message={}",
            field(payload, "code"),
            field(payload, "message")
        ),
        _ => compact_json(payload),
    };

    format!("{seq} {event_type}: {}", truncate(&detail, 260))
}

fn event_type(notification: &Value) -> Option<&str> {
    notification["params"]["type"].as_str()
}

fn event_seq(notification: &Value) -> Option<u64> {
    notification["params"]["seq"].as_u64()
}

fn assistant_iteration(notification: &Value) -> Option<u64> {
    notification["params"]["payload"]["iteration"].as_u64()
}

fn assistant_delta_text(notification: &Value) -> &str {
    notification["params"]["payload"]["text"]
        .as_str()
        .unwrap_or_default()
}

fn field(payload: &Value, key: &str) -> String {
    let value = &payload[key];
    if value.is_null() {
        return "-".to_owned();
    }
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    truncate(&text, 160)
}

fn nested_field(payload: &Value, outer: &str, inner: &str) -> String {
    let value = &payload[outer][inner];
    if value.is_null() {
        return "-".to_owned();
    }
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    truncate(&text, 160)
}

fn array_len_field(payload: &Value, key: &str) -> String {
    payload[key]
        .as_array()
        .map(|values| values.len().to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn compact_json(value: &Value) -> String {
    truncate(&value.to_string(), 220)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }

    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(windows)]
fn verification_command() -> String {
    "Write-Output 'VERIFY_OK_DEMO'; if ((Get-Content CLI_SMOKE.txt -Raw).Trim() -ne 'new') { exit 1 }"
        .to_owned()
}

#[cfg(not(windows))]
fn verification_command() -> String {
    "printf '%s\\n' 'VERIFY_OK_DEMO'; test \"$(cat CLI_SMOKE.txt)\" = new".to_owned()
}
