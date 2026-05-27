#![forbid(unsafe_code)]

// Result display tests. These tests are intentionally separate from normal
// development tests: they print a readable agent transcript for humans while
// leaving protocol-level streaming events unchanged.

use std::{
    collections::VecDeque,
    env,
    error::Error,
    fs,
    future::Future,
    process::{Command, Output},
    sync::{Arc, Mutex},
};

use prole_coder_agent_core::{
    cancellation::CancellationToken,
    context::{
        CachePlacement, ContextBuilder, ContextBuilderConfig, ContextCapsule, ContextItem,
        ContextItemKind, ContextManifestOmitted, ContextManifestReport,
    },
    provider::deepseek_api::ChatToolCall,
    provider::deepseek_api::{DEFAULT_API_BASE_URL, DEFAULT_MODEL},
    run_log::{RUN_LOG_MAX_ARRAY_ITEMS, RUN_LOG_MAX_STRING_BYTES, RunLogEvent, RunLogStore},
    test_helpers::{LIVE_TEST_FLAG, TestWorkspace, live_api_key, repo_root_from_crate_manifest},
    tool::{ToolName, find_builtin_tool, validate_tool_arguments},
    turn_loop::{
        AgentTurnInput, AgentTurnLoop, AgentTurnLoopError, TextRange, TurnAttachment, TurnProvider,
        TurnProviderFuture, TurnProviderRequest, TurnProviderResponse,
        turn_provider_response_stream,
    },
    workspace_manifest::{WorkspaceManifest, WorkspaceManifestConfig, build_workspace_manifest},
};
use serde_json::{Value, json};

#[test]
#[ignore = "result display test; run `cargo demo`"]
fn fixture_agent_interaction_transcript_demo() -> Result<(), Box<dyn Error>> {
    let workspace = TestWorkspace::with_preserve("fixture-agent-demo");
    workspace.write("CLI_SMOKE.txt", "old\n");

    let verify_command = verification_command();
    let output = Command::new(env!("CARGO_BIN_EXE_prole"))
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
    assert_final_text_contains(
        &demo_output.notifications,
        "Fixture provider applied CLI_SMOKE.txt",
    );
    assert_eq!(workspace.read("CLI_SMOKE.txt"), "new\n");

    Ok(())
}

#[test]
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, network access, and local cargo"]
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
name = "prole-coder-agent-demo"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    );
    workspace.write(
        "README.md",
        "Small repository used to demonstrate a real ProleCoder agent turn.\n",
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
        assert_eq!(super::greeting(), "hello from ProleCoder demo");
    }
}
"#,
    );
    workspace.git_init();
    workspace.git_commit_all("initial");

    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("PROLE_CODER_DEMO_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let output = Command::new(env!("CARGO_BIN_EXE_prole"))
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
                "This is a live ProleCoder agent interaction demo. ",
                "You MUST use tools before answering. First call read_file for README.md and src/lib.rs. ",
                "Then use apply_patch to edit only src/lib.rs so greeting() returns exactly ",
                "hello from ProleCoder demo. When calling apply_patch, expectedFiles MUST be a JSON array ",
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
    assert_event(&demo_output.notifications, "provider.completed");
    assert_event(&demo_output.notifications, "verification.completed");
    assert_event(&demo_output.notifications, "run.completed");
    assert_final_text_contains(&demo_output.notifications, "OK_AGENT_DEMO");
    assert!(
        workspace
            .read("src/lib.rs")
            .contains("hello from ProleCoder demo")
    );

    Ok(())
}

#[test]
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, network access, and local cargo"]
fn live_deepseek_agent_random_story_demo() -> Result<(), Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping random live DeepSeek agent demo: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(());
    }

    let api_key = live_api_key(repo_root_from_crate_manifest(env!("CARGO_MANIFEST_DIR")))?;
    let scenario = live_random_scenario();
    let workspace = TestWorkspace::with_preserve("live-agent-random-story-demo");
    write_live_random_workspace(&workspace, &scenario);
    workspace.git_init();
    workspace.git_commit_all("initial random story demo");

    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("PROLE_CODER_DEMO_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let task = format!(
        "This is a longer live ProleCoder random story demo. Scenario seed: {seed}. \
         You MUST use tools before answering. First call read_file for README.md, docs/task.md, \
         Cargo.toml, src/lib.rs, tests/behavior.rs, and CHANGELOG.md. Then call search for \
         RANDOM_DEMO_TARGET. Then use apply_patch to edit exactly src/lib.rs and CHANGELOG.md. \
         When calling apply_patch, expectedFiles MUST be a JSON array exactly like \
         [\"src/lib.rs\",\"CHANGELOG.md\"], not a quoted string. Do not call shell; the harness \
         will run cargo test. Make the tests pass, update the changelog bullet exactly as docs/task.md \
         requests, and finish with a concise answer containing OK_RANDOM_AGENT_DEMO and seed {seed}.",
        seed = scenario.seed
    );

    let output = Command::new(env!("CARGO_BIN_EXE_prole"))
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
            "demo_live_random_agent",
            "--turn-id",
            "demo_live_random_agent_turn",
            "--max-model-turns",
            "8",
            "--max-output-tokens",
            "1400",
            "--verify",
            "cargo test --quiet",
            "--verify-timeout-ms",
            "120000",
            "--",
        ])
        .arg(task)
        .output()?;

    let demo_output = successful_agent_events(output, &workspace, "demo_live_random_agent")?;
    assert!(!demo_output.stdout.contains(&api_key));
    assert!(!demo_output.stderr.contains(&api_key));
    print_agent_transcript(
        "Live DeepSeek Agent Random Story Demo",
        &workspace,
        "demo_live_random_agent",
        &demo_output.notifications,
        &["docs/task.md", "src/lib.rs", "CHANGELOG.md"],
    );

    assert_event(&demo_output.notifications, "tool.completed");
    assert_event(&demo_output.notifications, "provider.completed");
    assert_event(&demo_output.notifications, "verification.completed");
    assert_event(&demo_output.notifications, "run.completed");
    assert_final_text_contains(&demo_output.notifications, "OK_RANDOM_AGENT_DEMO");
    assert!(workspace.read("src/lib.rs").contains(&scenario.greeting));
    assert!(
        workspace
            .read("src/lib.rs")
            .contains(scenario.release_keyword)
    );
    assert!(
        workspace
            .read("CHANGELOG.md")
            .contains(&scenario.changelog_line)
    );

    Ok(())
}
#[test]
#[ignore = "result display test; run `cargo demo-context`"]
fn context_capsule_structure_demo() -> Result<(), Box<dyn Error>> {
    let workspace = context_demo_workspace("context-capsule-demo");
    let (manifest, capsule) = build_demo_context_capsule(&workspace)?;

    print_context_capsule_demo(
        "Context Capsule Structure Demo",
        &workspace,
        &manifest,
        &capsule,
        false,
    );

    let payload = capsule.context_built_payload();
    assert!(payload["stablePrefixHash"].as_str().is_some());
    assert!(payload["manifest"]["manifestHash"].as_str().is_some());
    assert!(
        payload["includedSources"]
            .as_array()
            .is_some_and(|sources| sources.iter().any(|source| source["kind"] == "file"))
    );
    assert!(
        payload["omittedSources"]
            .as_array()
            .is_some_and(|sources| !sources.is_empty())
    );

    Ok(())
}

#[test]
#[ignore = "result display test; run `cargo demo-context-visual`"]
fn context_capsule_visualization_demo() -> Result<(), Box<dyn Error>> {
    let workspace = context_demo_workspace("context-visual-demo");
    let (manifest, capsule) = build_demo_context_capsule(&workspace)?;

    print_context_capsule_demo(
        "Context Capsule Visualization Demo",
        &workspace,
        &manifest,
        &capsule,
        true,
    );

    let visualization = context_visualization(&capsule);
    assert!(visualization.contains("stable_prefix"));
    assert!(visualization.contains("dynamic_prelude"));
    assert!(visualization.contains("turn_suffix"));

    Ok(())
}

#[test]
#[ignore = "result display test; run `cargo demo-truncation`"]
fn run_log_truncation_demo() -> Result<(), Box<dyn Error>> {
    let workspace = TestWorkspace::with_preserve("run-log-truncation-demo");
    let store = RunLogStore::new(workspace.path())?;
    let mut run = store.create_run("demo_run_log_truncation")?;
    let long_stdout = "A".repeat(RUN_LOG_MAX_STRING_BYTES + 128);
    let many_matches = (0..RUN_LOG_MAX_ARRAY_ITEMS + 4)
        .map(|index| json!({ "line": index + 1, "text": format!("match {index}") }))
        .collect::<Vec<_>>();

    run.append(
        "tool.completed",
        Some("demo_truncation_turn".to_owned()),
        json!({
            "name": "search",
            "status": "ok",
            "stdout": long_stdout,
            "stderr": "",
            "matches": many_matches,
            "summary": "This payload intentionally contains a huge stdout and too many matches."
        }),
    )?;

    let events = store.load_run("demo_run_log_truncation")?;
    print_run_log_truncation_demo(
        "Run Log Truncation Demo",
        &workspace,
        "demo_run_log_truncation",
        &events,
    );

    let payload = &events[0].payload;
    assert_eq!(
        payload["stdout"]
            .as_str()
            .expect("stdout should stay a string")
            .len(),
        RUN_LOG_MAX_STRING_BYTES
    );
    assert_eq!(payload["stderr"], "");
    assert!(payload.get("missingField").is_none());
    assert!(
        payload["runLogTruncation"]
            .as_array()
            .is_some_and(|truncation| truncation.iter().any(|entry| {
                entry["path"] == "$.stdout" && entry["reason"] == "max_string_bytes"
            }))
    );
    assert!(
        payload["runLogTruncation"]
            .as_array()
            .is_some_and(|truncation| truncation.iter().any(|entry| {
                entry["path"] == "$.matches" && entry["reason"] == "max_array_items"
            }))
    );

    Ok(())
}

#[test]
#[ignore = "result display test; run `cargo demo-schema`"]
fn tool_schema_validation_demo() -> Result<(), Box<dyn Error>> {
    let workspace = TestWorkspace::with_preserve("tool-schema-demo");
    workspace.write("README.md", "schema demo\n");
    let store = RunLogStore::new(workspace.path())?;
    let mut run = store.create_run("demo_tool_schema")?;
    let provider = ScriptedProvider::new(vec![TurnProviderResponse::tool_calls(
        None,
        Some("I should read README.md, but my tool arguments contain an extra field.".to_owned()),
        vec![ChatToolCall::function(
            "call_schema_invalid",
            "read_file",
            r#"{"path":"README.md","unexpected":true}"#,
        )],
    )]);
    let mut loop_runner = AgentTurnLoop::new(workspace.path(), provider)?;

    let error = block_on_turn(loop_runner.run_turn(
        AgentTurnInput::new("demo_schema_turn", "Show tool schema validation"),
        &mut run,
    ))
    .expect_err("schema demo should fail before typed deserialization");

    print_expected_error("Tool Schema Validation Demo", &error);
    let events = store.load_run("demo_tool_schema")?;
    print_run_log_event_demo(
        "Tool Schema Validation Demo Events",
        &workspace,
        "demo_tool_schema",
        &events,
    );

    let read_file = find_builtin_tool(ToolName::ReadFile.as_str()).expect("read_file should exist");
    let schema_error = validate_tool_arguments(
        read_file,
        &json!({ "path": "README.md", "unexpected": true }),
    )
    .expect_err("extra property should fail schema validation");
    println!();
    println!("--- Direct schema validator result ---");
    println!("path: {}", schema_error.path());
    println!("detail: {}", schema_error.detail());

    assert!(matches!(
        error,
        AgentTurnLoopError::InvalidToolArgumentSchema { .. }
    ));
    assert!(events.iter().any(|event| event.event_type == "run.failed"
        && event.payload["code"] == "E_INVALID_TOOL_ARGUMENTS"));
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "tool.requested")
    );

    Ok(())
}

#[test]
#[ignore = "result display test; run `cargo demo-attachment`"]
fn attachment_context_demo() -> Result<(), Box<dyn Error>> {
    let workspace = TestWorkspace::with_preserve("attachment-context-demo");
    workspace.write("README.md", "attached README\nsecond line\n");
    workspace.write("src/lib.rs", "pub fn demo() {}\n");
    workspace.write("docs/notes.md", "manual note\n");
    let store = RunLogStore::new(workspace.path())?;
    let mut run = store.create_run("demo_attachment_context")?;
    let (provider, requests) =
        ScriptedProvider::with_request_recorder(vec![TurnProviderResponse::final_text(
            "attachments received.",
        )]);
    let mut loop_runner = AgentTurnLoop::new(workspace.path(), provider)?;
    let range = TextRange::new(1, 1, 1, 18);

    block_on_turn(
        loop_runner.run_turn(
            AgentTurnInput::new("demo_attachment_turn", "Use all attachment kinds")
                .with_attachment(TurnAttachment::file("README.md"))
                .with_attachment(TurnAttachment::selection(
                    "src/lib.rs",
                    range,
                    "pub fn demo() {}",
                ))
                .with_attachment(TurnAttachment::explicit_content(
                    "acceptance: mention every attachment kind",
                ))
                .with_attachment(TurnAttachment::diagnostic(
                    "src/lib.rs",
                    range,
                    "warning: demo is unused",
                )),
            &mut run,
        ),
    )?;

    let events = store.load_run("demo_attachment_context")?;
    print_run_log_event_demo(
        "Attachment Context Demo",
        &workspace,
        "demo_attachment_context",
        &events,
    );

    let prompt = recorded_prompt(&requests);
    println!();
    println!("--- Provider prompt excerpt ---");
    println!("{}", truncate(&prompt, 1600));

    let context_built = events
        .iter()
        .find(|event| event.event_type == "context.built")
        .expect("context.built should be emitted");
    for kind in ["file", "selection", "explicit_content", "diagnostic"] {
        assert!(
            context_built.payload["includedSources"]
                .as_array()
                .is_some_and(|sources| sources.iter().any(|source| source["kind"] == kind)),
            "context should include {kind} attachment source"
        );
        assert!(
            prompt.contains(&format!("Attachment-Kind: {kind}")),
            "prompt should show attachment kind {kind}"
        );
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct LiveRandomScenario {
    seed: u64,
    theme: &'static str,
    release_keyword: &'static str,
    greeting: String,
    release_note: String,
    changelog_line: String,
}

fn live_random_scenario() -> LiveRandomScenario {
    let seed = env::var("PROLE_CODER_DEMO_SEED")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0)
                ^ u64::from(std::process::id())
        });
    let themes = [
        "forge",
        "harbor",
        "observatory",
        "workshop",
        "archive",
        "garden",
    ];
    let release_keywords = [
        "steady",
        "curious",
        "auditable",
        "bright",
        "patient",
        "useful",
    ];
    let theme = themes[(seed as usize) % themes.len()];
    let release_keyword =
        release_keywords[((seed / themes.len() as u64) as usize) % release_keywords.len()];
    let short_seed = seed % 10_000;
    let greeting = format!("Hello from the {theme} scenario #{short_seed}");
    let release_note = format!(
        "OK_RANDOM_AGENT_DEMO seed {short_seed}: {theme} scenario stays {release_keyword}."
    );
    let changelog_line = format!(
        "- random live demo seed {short_seed}: {theme} / {release_keyword} / OK_RANDOM_AGENT_DEMO"
    );

    LiveRandomScenario {
        seed,
        theme,
        release_keyword,
        greeting,
        release_note,
        changelog_line,
    }
}

fn write_live_random_workspace(workspace: &TestWorkspace, scenario: &LiveRandomScenario) {
    workspace.write(
        "Cargo.toml",
        r#"[package]
name = "prole-live-random-demo"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    );
    workspace.write(
        "README.md",
        "Small randomized repository used to demonstrate a longer real ProleCoder agent turn.\n",
    );
    workspace.write(
        "docs/task.md",
        &format!(
            "# Random live demo task\n\n\
             Seed: {seed}\n\
             Theme: {theme}\n\
             Release keyword: {keyword}\n\n\
             Required source changes:\n\
             - `greeting()` must return exactly `{greeting}`.\n\
             - `selected_theme()` must return exactly `{theme}`.\n\
             - `release_note()` must return exactly `{release_note}`.\n\n\
             Required changelog change:\n\
             - Replace the TODO bullet in `CHANGELOG.md` with exactly `{changelog_line}`.\n",
            seed = scenario.seed,
            theme = scenario.theme,
            keyword = scenario.release_keyword,
            greeting = scenario.greeting,
            release_note = scenario.release_note,
            changelog_line = scenario.changelog_line,
        ),
    );
    workspace.write(
        "src/lib.rs",
        r#"// RANDOM_DEMO_TARGET: the live agent should find this marker before patching.

pub fn greeting() -> &'static str {
    "old randomized greeting"
}

pub fn selected_theme() -> &'static str {
    "unset"
}

pub fn release_note() -> &'static str {
    "TODO: release note"
}
"#,
    );
    workspace.write(
        "tests/behavior.rs",
        &format!(
            r#"use prole_live_random_demo::{{greeting, release_note, selected_theme}};

#[test]
fn random_scenario_matches_task_document() {{
    assert_eq!(greeting(), "{greeting}");
    assert_eq!(selected_theme(), "{theme}");
    assert_eq!(release_note(), "{release_note}");
    assert!(release_note().contains("{keyword}"));
    assert!(release_note().contains("OK_RANDOM_AGENT_DEMO"));
}}
"#,
            greeting = scenario.greeting,
            theme = scenario.theme,
            release_note = scenario.release_note,
            keyword = scenario.release_keyword,
        ),
    );
    workspace.write(
        "CHANGELOG.md",
        "# Changelog\n\n- TODO: describe the random live demo result.\n",
    );
}
struct AgentDemoOutput {
    notifications: Vec<Value>,
    stdout: String,
    stderr: String,
}

type RequestRecorder = Arc<Mutex<Vec<TurnProviderRequest>>>;

#[derive(Debug)]
struct ScriptedProvider {
    responses: VecDeque<TurnProviderResponse>,
    requests: Option<RequestRecorder>,
}

impl ScriptedProvider {
    fn new(responses: Vec<TurnProviderResponse>) -> Self {
        Self {
            responses: responses.into(),
            requests: None,
        }
    }

    fn with_request_recorder(responses: Vec<TurnProviderResponse>) -> (Self, RequestRecorder) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                responses: responses.into(),
                requests: Some(Arc::clone(&requests)),
            },
            requests,
        )
    }
}

impl TurnProvider for ScriptedProvider {
    fn complete_stream(&mut self, request: TurnProviderRequest) -> TurnProviderFuture<'_> {
        Box::pin(async move {
            if let Some(requests) = &self.requests {
                requests
                    .lock()
                    .expect("request recorder lock should not be poisoned")
                    .push(request);
            }
            let response = self.responses.pop_front().ok_or_else(|| {
                prole_coder_agent_core::turn_loop::TurnProviderError::new(
                    "scripted provider has no response",
                )
            })?;
            Ok(turn_provider_response_stream(response))
        })
    }
}

fn block_on_turn<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("demo runtime should build")
        .block_on(future)
}

fn context_demo_workspace(name: &str) -> TestWorkspace {
    let workspace = TestWorkspace::with_preserve(name);
    workspace.write(
        "README.md",
        "# Demo crate\n\nSmall workspace for Context Capsule demos.\n",
    );
    workspace.write(
        "Cargo.toml",
        "[package]\nname = \"context-demo\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    );
    workspace.write("src/lib.rs", "pub fn answer() -> u32 {\n    42\n}\n");
    workspace.write("src/main.rs", "fn main() {\n    println!(\"demo\");\n}\n");
    workspace.write(
        "tests/smoke.rs",
        "#[test]\nfn smoke() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    workspace.write("docs/design.md", "Context notes for the demo.\n");
    workspace.write("config/settings.json", "{\"mode\":\"demo\"}\n");
    workspace.write(
        ".secrets/deepseek-api-key",
        "demo secret value that must not appear",
    );
    workspace
}

fn build_demo_context_capsule(
    workspace: &TestWorkspace,
) -> Result<(WorkspaceManifest, ContextCapsule), Box<dyn Error>> {
    let manifest = build_workspace_manifest(
        workspace.path(),
        None,
        WorkspaceManifestConfig::new(4),
        &CancellationToken::new(),
    )?;
    let manifest_summary = manifest.summary_markdown();
    let capsule = ContextBuilder::new(
        ContextBuilderConfig::new(2_400).with_stable_prefix_budget_ratio_ppm(800_000),
    )
    .with_manifest_report(context_manifest_report(&manifest))
    .with_item(ContextItem::project_rules(
        "Keep demos deterministic and never include local secrets.",
        "project demo rules",
    ))
    .with_item(ContextItem::workspace_manifest(
        manifest_summary,
        "stable workspace manifest summary",
    ))
    .with_item(ContextItem::required(
        ContextItemKind::Diagnostic,
        "src/lib.rs:1:1 warning: demo function has no caller",
        "current diagnostics",
    ))
    .with_item(ContextItem::file(
        "src/lib.rs",
        workspace.read("src/lib.rs"),
        "selected implementation file",
    ))
    .with_item(ContextItem::optional(
        ContextItemKind::Other,
        "optional scratch note\n".repeat(260),
        "large optional note used to demonstrate omittedSources",
    ))
    .with_item(ContextItem::user_task(
        "Explain the workspace shape and why the selected Rust file is relevant.",
    ))
    .build()?;

    Ok((manifest, capsule))
}

fn context_manifest_report(manifest: &WorkspaceManifest) -> ContextManifestReport {
    ContextManifestReport {
        manifest_hash: manifest.manifest_hash.clone(),
        max_entries: manifest.max_entries,
        total_discovered_files: manifest.total_discovered_files,
        included_files: manifest.included_files,
        omitted: manifest
            .omitted
            .iter()
            .map(|omitted| ContextManifestOmitted {
                reason: omitted.reason.as_str().to_owned(),
                count: omitted.count,
            })
            .collect(),
    }
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

fn assert_final_text_contains(notifications: &[Value], expected: &str) {
    let assistant_text = visible_assistant_text(notifications);
    let summary = run_completed_summary(notifications).unwrap_or_default();

    assert!(
        assistant_text.contains(expected),
        "final assistant text should contain {expected:?}, got {assistant_text:?}"
    );
    assert!(
        summary.contains(expected),
        "run.completed summary should contain {expected:?}, got {summary:?}"
    );
}

fn visible_assistant_text(notifications: &[Value]) -> String {
    notifications
        .iter()
        .filter(|notification| event_type(notification) == Some("assistant.delta"))
        .filter_map(|notification| notification["params"]["payload"]["text"].as_str())
        .collect::<String>()
}

fn run_completed_summary(notifications: &[Value]) -> Option<&str> {
    notifications
        .iter()
        .filter(|notification| event_type(notification) == Some("run.completed"))
        .find_map(|notification| notification["params"]["payload"]["summary"].as_str())
}

fn print_context_capsule_demo(
    title: &str,
    workspace: &TestWorkspace,
    manifest: &WorkspaceManifest,
    capsule: &ContextCapsule,
    include_visualization: bool,
) {
    println!();
    println!("=== {title} ===");
    println!("workspace: {}", workspace.path_str());
    println!();
    println!("--- Workspace manifest summary ---");
    println!("{}", manifest.summary_markdown().trim_end());
    println!();
    println!("--- Context sections ---");
    for section in &capsule.sections {
        println!(
            "{}: tokens={} items={}",
            placement_name(section.placement),
            section.tokens,
            section.items.len()
        );
        for item in &section.items {
            println!(
                "  - kind={:?} path={} tokens={} required={} reason={}",
                item.source.kind,
                item.source.path.as_deref().unwrap_or("-"),
                item.tokens,
                item.source.required,
                item.reason
            );
        }
    }
    println!();
    println!("--- Included sources ---");
    for source in &capsule.token_report.included_sources {
        println!(
            "- kind={:?} path={} tokens={} reason={}",
            source.source.kind,
            source.source.path.as_deref().unwrap_or("-"),
            source.tokens,
            source.reason
        );
    }
    println!();
    println!("--- Omitted sources ---");
    for source in &capsule.token_report.omitted_sources {
        println!(
            "- kind={:?} tokens={} reason={:?}",
            source.source.kind, source.estimated_tokens, source.omission_reason
        );
    }
    if include_visualization {
        println!();
        println!("--- Context Capsule Visualization ---");
        println!("{}", context_visualization(capsule));
    }
    println!();
    println!("--- Raw context.built ---");
    println!(
        "{}",
        serde_json::to_string_pretty(&capsule.context_built_payload())
            .expect("context payload should serialize")
    );
}

fn context_visualization(capsule: &ContextCapsule) -> String {
    let total = capsule
        .sections
        .iter()
        .map(|section| section.tokens)
        .sum::<u64>()
        .max(1);
    let width = 36_u64;
    let mut output = String::new();
    output.push_str(&format!(
        "inputTokens={} maxInputTokens={} stablePrefixHash={}\n",
        capsule.token_report.input_tokens,
        capsule.token_report.max_input_tokens,
        capsule.stable_prefix_hash()
    ));
    for section in &capsule.sections {
        let bar_len = usize::try_from((section.tokens * width).div_ceil(total))
            .expect("visualization bar width should fit usize");
        let empty_len = usize::try_from(width)
            .expect("visualization width should fit usize")
            .saturating_sub(bar_len);
        output.push_str(&format!(
            "{:<16} {:>6} |{}{}| items={}\n",
            placement_name(section.placement),
            section.tokens,
            "#".repeat(bar_len),
            ".".repeat(empty_len),
            section.items.len()
        ));
    }
    output
}

fn placement_name(placement: CachePlacement) -> &'static str {
    match placement {
        CachePlacement::StablePrefix => "stable_prefix",
        CachePlacement::DynamicPrelude => "dynamic_prelude",
        CachePlacement::TurnSuffix => "turn_suffix",
    }
}

fn print_run_log_event_demo(
    title: &str,
    workspace: &TestWorkspace,
    run_id: &str,
    events: &[RunLogEvent],
) {
    println!();
    println!("=== {title} ===");
    println!("workspace: {}", workspace.path_str());
    println!();
    println!("--- Run log events ---");
    let notifications = run_log_events_to_notifications(events);
    print_agent_events(&notifications);
    println!();
    println!("--- Raw events ---");
    for event in events {
        println!(
            "{}",
            serde_json::to_string_pretty(event).expect("event should serialize")
        );
    }
    let summary_path = workspace
        .path()
        .join(".prole-coder")
        .join("runs")
        .join(run_id)
        .join("summary.json");
    if let Ok(summary) = fs::read_to_string(summary_path) {
        println!();
        println!("--- Run summary ---");
        println!("{}", summary.trim_end());
    }
}

fn print_run_log_truncation_demo(
    title: &str,
    workspace: &TestWorkspace,
    run_id: &str,
    events: &[RunLogEvent],
) {
    println!();
    println!("=== {title} ===");
    println!("workspace: {}", workspace.path_str());
    println!();
    println!("--- Run log events ---");
    let notifications = run_log_events_to_notifications(events);
    print_agent_events(&notifications);

    let Some(event) = events.first() else {
        println!("no events recorded");
        return;
    };
    let payload = &event.payload;
    let stdout = payload["stdout"].as_str().unwrap_or("");
    let matches = payload["matches"].as_array().cloned().unwrap_or_default();
    let matches_preview = matches.into_iter().take(3).collect::<Vec<_>>();
    let stdout_preview = stdout.chars().take(80).collect::<String>();
    let snapshot = json!({
        "name": payload["name"],
        "status": payload["status"],
        "stdoutPreview": stdout_preview,
        "stdoutStoredBytes": stdout.len(),
        "stderr": payload["stderr"],
        "matchesStored": payload["matches"].as_array().map_or(0, Vec::len),
        "matchesPreview": matches_preview,
        "missingFieldPresent": payload.get("missingField").is_some(),
        "runLogTruncation": payload["runLogTruncation"],
    });

    println!();
    println!("--- Truncation snapshot ---");
    println!(
        "{}",
        serde_json::to_string_pretty(&snapshot).expect("snapshot should serialize")
    );

    print_run_summary(workspace, run_id);
}

fn print_run_summary(workspace: &TestWorkspace, run_id: &str) {
    let summary_path = workspace
        .path()
        .join(".prole-coder")
        .join("runs")
        .join(run_id)
        .join("summary.json");
    if let Ok(summary) = fs::read_to_string(summary_path) {
        println!();
        println!("--- Run summary ---");
        println!("{}", summary.trim_end());
    }
}

fn run_log_events_to_notifications(events: &[RunLogEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            json!({
                "jsonrpc": "2.0",
                "method": "agent.event",
                "params": {
                    "seq": event.seq,
                    "timeUnixMs": event.time_unix_ms,
                    "type": event.event_type,
                    "runId": event.run_id,
                    "turnId": event.turn_id,
                    "payload": event.payload,
                }
            })
        })
        .collect()
}

fn print_expected_error(title: &str, error: &AgentTurnLoopError) {
    println!();
    println!("=== {title} ===");
    println!("expected error code: {}", error.code());
    println!("expected error: {error}");
}

fn recorded_prompt(requests: &RequestRecorder) -> String {
    let requests = requests
        .lock()
        .expect("request recorder lock should not be poisoned");
    requests
        .first()
        .and_then(|request| request.messages.first())
        .and_then(|message| message.content.as_deref())
        .unwrap_or_default()
        .to_owned()
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
            "temporary; set PROLE_CODER_KEEP_DEMO_WORKSPACE=1 to keep it"
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
        .join(".prole-coder")
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
        "provider.completed" => format!(
            "iteration={} model={} finish={} durationMs={} prompt={} completion={} total={} cacheHit={} cacheMiss={} chunks={} toolDeltas={}",
            field(payload, "iteration"),
            field(payload, "model"),
            field(payload, "finishReason"),
            field(payload, "durationMs"),
            nested_field(payload, "usage", "promptTokens"),
            nested_field(payload, "usage", "completionTokens"),
            nested_field(payload, "usage", "totalTokens"),
            nested_field(payload, "usage", "promptCacheHitTokens"),
            nested_field(payload, "usage", "promptCacheMissTokens"),
            nested_field(payload, "streaming", "chunkCount"),
            nested_field(payload, "streaming", "toolCallDeltaCount")
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
