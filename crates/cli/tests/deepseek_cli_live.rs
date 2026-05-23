#![forbid(unsafe_code)]

use std::{env, error::Error, fs, io, path::PathBuf, process::Command};

use deepseek_coder_agent_core::provider::deepseek_api::{DEFAULT_API_BASE_URL, DEFAULT_MODEL};
use serde_json::Value;

const LIVE_TEST_FLAG: &str = "DEEPSEEK_CODER_LIVE_TESTS";
const LIVE_API_KEY_FILE: &str = ".secrets/deepseek-api-key";
const API_KEY_PLACEHOLDER: &str = "<put-your-deepseek-api-key-here>";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("cli crate must be nested under crates/")
        .to_path_buf()
}

fn live_api_key() -> Result<String, Box<dyn Error>> {
    if let Ok(api_key) = env::var("DEEPSEEK_API_KEY") {
        let api_key = api_key.trim();
        if !api_key.is_empty() && api_key != API_KEY_PLACEHOLDER {
            return Ok(api_key.to_owned());
        }
    }

    let api_key_path = workspace_root().join(LIVE_API_KEY_FILE);
    let api_key = fs::read_to_string(api_key_path).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!(
                "DEEPSEEK_API_KEY is not set and {LIVE_API_KEY_FILE} could not be read: {source}"
            ),
        )
    })?;
    let api_key = api_key.trim();
    if api_key.is_empty() || api_key == API_KEY_PLACEHOLDER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("put a DeepSeek API key in {LIVE_API_KEY_FILE} or set DEEPSEEK_API_KEY"),
        )
        .into());
    }

    Ok(api_key.to_owned())
}

#[test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, API key, and network access"]
fn live_deepseek_cli_streaming_smoke_test() -> Result<(), Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek CLI test: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(());
    }

    let api_key = live_api_key()?;
    let workspace = TestWorkspace::new();
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

struct TestWorkspace {
    path: PathBuf,
    path_string: String,
}

impl TestWorkspace {
    fn new() -> Self {
        let unique = format!(
            "deepseek-coder-cli-live-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        );
        let path = env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("temp workspace should be created");
        let path_string = path.display().to_string();
        Self { path, path_string }
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
