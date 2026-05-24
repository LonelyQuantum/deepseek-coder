#![forbid(unsafe_code)]

use std::{env, error::Error, process::Command};

use deepseek_coder_agent_core::{
    provider::deepseek_api::{DEFAULT_API_BASE_URL, DEFAULT_MODEL, DeepSeekModelId},
    run_log::RunLogStore,
    test_helpers::{LIVE_TEST_FLAG, TestWorkspace, live_api_key, repo_root_from_crate_manifest},
};
use serde_json::Value;

#[test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, API key, and network access"]
fn live_deepseek_cli_streaming_smoke_test() -> Result<(), Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek CLI test: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(());
    }

    let api_key = live_api_key(repo_root_from_crate_manifest(env!("CARGO_MANIFEST_DIR")))?;
    let workspace = TestWorkspace::new("cli-live");
    workspace.write("README.md", "live streaming smoke workspace\n");
    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());

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
            "ask",
            "--json",
            "--workspace",
            workspace.path_str(),
            "--run-id",
            "run_cli_live_stream",
            "--turn-id",
            "turn_cli_live_stream",
            "--max-model-turns",
            "1",
            "--max-output-tokens",
            "128",
            "--",
            "Do not call tools. Reply with the exact text OK_STREAMING_CLI.",
        ])
        .output()?;

    assert!(
        output.status.success(),
        "CLI live streaming failed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    assert!(!stdout.contains(&api_key));
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

    let streamed_delta = notifications.iter().any(|value| {
        value["params"]["type"] == "assistant.delta"
            && value["params"]["payload"]["stream"] == true
            && value["params"]["payload"]["text"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
    });
    assert!(
        streamed_delta,
        "live CLI run should emit at least one streaming assistant.delta"
    );

    let completed = notifications
        .iter()
        .find(|value| value["params"]["type"] == "run.completed")
        .ok_or("live CLI run should emit run.completed")?;
    let summary = completed["params"]["payload"]["summary"]
        .as_str()
        .ok_or("run.completed summary should be a string")?;
    assert!(
        summary.contains("OK_STREAMING_CLI"),
        "live CLI final summary should contain OK_STREAMING_CLI, got: {summary}"
    );

    Ok(())
}

#[test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, API key, network access, and local cargo"]
fn live_deepseek_cli_real_repo_acceptance_test() -> Result<(), Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek CLI test: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(());
    }

    let api_key = live_api_key(repo_root_from_crate_manifest(env!("CARGO_MANIFEST_DIR")))?;
    let workspace = TestWorkspace::new("cli-live-real-repo");
    workspace.write(
        "Cargo.toml",
        r#"[package]
name = "deepseek-coder-live-acceptance"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    );
    workspace.write(
        "README.md",
        "Tiny Rust repository for the deepseek-coder live CLI acceptance test.\n",
    );
    workspace.write(
        "src/lib.rs",
        r#"pub fn greeting() -> &'static str {
    "OLD_GREETING"
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_cli_acceptance_marker() {
        assert_eq!(super::greeting(), "HELLO_FROM_DEEPSEEK_CLI");
    }
}
"#,
    );
    workspace.git_init();
    workspace.git_commit_all("initial");

    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("DEEPSEEK_CLI_ACCEPTANCE_MODEL")
        .unwrap_or_else(|_| DeepSeekModelId::V4_FLASH.to_owned());
    let verify_command = cargo_test_command();

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
            "run_cli_live_real_repo",
            "--turn-id",
            "turn_cli_live_real_repo",
            "--max-model-turns",
            "6",
            "--max-output-tokens",
            "768",
            "--verify",
            verify_command.as_str(),
            "--verify-timeout-ms",
            "120000",
            "--",
            concat!(
                "This is a live CLI acceptance test in a tiny Rust git repository. ",
                "You MUST use tools before answering. Do not write prose in the first assistant response; ",
                "the first response must call read_file for README.md and src/lib.rs. ",
                "After reading the files, use apply_patch to change only src/lib.rs so greeting() returns exactly ",
                "HELLO_FROM_DEEPSEEK_CLI. Do not call shell; the harness will run cargo test after you finish. ",
                "Do not finish until the patch has been applied. ",
                "After the patch is applied, reply with a short final answer containing the exact token OK_REAL_REPO_CLI."
            ),
        ])
        .output()?;

    assert!(
        output.status.success(),
        "CLI live real-repo acceptance failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(!stdout.contains(&api_key));
    assert!(!stderr.contains(&api_key));
    assert!(
        workspace
            .read("src/lib.rs")
            .contains("HELLO_FROM_DEEPSEEK_CLI"),
        "src/lib.rs should contain the live acceptance marker"
    );

    let notifications = json_lines(&stdout)?;
    assert!(
        notifications
            .iter()
            .all(|value| value["method"] == "agent.event"),
        "live acceptance stdout should contain only agent.event notifications"
    );
    assert!(notifications.iter().any(|value| {
        value["params"]["type"] == "tool.completed"
            && value["params"]["payload"]["name"] == "read_file"
            && value["params"]["payload"]["status"] == "ok"
    }));
    assert!(notifications.iter().any(|value| {
        value["params"]["type"] == "tool.completed"
            && value["params"]["payload"]["name"] == "apply_patch"
            && value["params"]["payload"]["status"] == "ok"
    }));
    assert!(notifications.iter().any(|value| {
        value["params"]["type"] == "verification.completed"
            && value["params"]["payload"]["status"] == "passed"
    }));
    let completed = notifications
        .iter()
        .find(|value| value["params"]["type"] == "run.completed")
        .ok_or("live acceptance should emit run.completed")?;
    assert_eq!(
        completed["params"]["payload"]["changedFiles"],
        serde_json::json!(["src/lib.rs"])
    );
    assert!(
        completed["params"]["payload"]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("OK_REAL_REPO_CLI")),
        "live acceptance final summary should contain OK_REAL_REPO_CLI"
    );

    let store = RunLogStore::new(workspace.path())?;
    let events = store.load_run("run_cli_live_real_repo")?;
    assert_eq!(notifications.len(), events.len());
    assert!(events.iter().any(|event| {
        event.event_type == "verification.completed" && event.payload["status"] == "passed"
    }));

    Ok(())
}

fn json_lines(output: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    output
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).map_err(Into::into))
        .collect()
}

fn cargo_test_command() -> String {
    "cargo test --quiet".to_owned()
}
