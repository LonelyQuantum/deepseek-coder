use std::{env, error::Error, fs, io, path::PathBuf, time::Duration};

use deepseek_coder_agent_core::{
    provider::deepseek_api::{
        ChatFunctionDefinition, ChatMessage, ChatTool, DEFAULT_API_BASE_URL, DEFAULT_MODEL,
        DeepSeekApiAdapter, DeepSeekApiConfig, StreamEvent, ThinkingConfig,
    },
    reasoning::{ReasoningContentState, ReasoningContentStateMachine},
};
use futures_util::StreamExt;

const LIVE_TEST_FLAG: &str = "DEEPSEEK_CODER_LIVE_TESTS";
const LIVE_API_KEY_FILE: &str = ".secrets/deepseek-api-key";
const API_KEY_PLACEHOLDER: &str = "<put-your-deepseek-api-key-here>";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("agent-core crate must be nested under crates/")
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

fn live_config() -> Result<DeepSeekApiConfig, Box<dyn Error>> {
    let api_key = live_api_key()?;
    let base_url =
        env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let model = env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    Ok(DeepSeekApiConfig::new(api_key, base_url, model)?)
}

fn live_adapter() -> Result<Option<DeepSeekApiAdapter>, Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek API test: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(None);
    }

    let config = live_config()?.with_timeout(Duration::from_secs(120));
    Ok(Some(DeepSeekApiAdapter::new(config)?))
}

fn live_reasoning_probe_tool() -> ChatTool {
    ChatTool::function(ChatFunctionDefinition {
        name: "get_live_reasoning_fixture".to_owned(),
        description: "Return the fixed fixture value for the reasoning replay live test."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {},
            "required": []
        }),
    })
}

#[tokio::test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, API key, and network access"]
async fn live_chat_completion_smoke_test() -> Result<(), Box<dyn Error>> {
    let Some(adapter) = live_adapter()? else {
        return Ok(());
    };

    let request = adapter
        .new_chat_request(vec![ChatMessage::user(
            "Reply with one short sentence confirming that the API is reachable.",
        )])?
        .with_thinking(ThinkingConfig::disabled())
        .with_max_tokens(64);

    let response = adapter.create_chat_completion(request).await?;
    let choice = response
        .choices
        .first()
        .ok_or("live response must contain at least one choice")?;

    assert!(
        choice
            .message
            .content
            .as_deref()
            .is_some_and(|content| !content.trim().is_empty())
            || choice
                .message
                .reasoning_content
                .as_deref()
                .is_some_and(|content| !content.trim().is_empty()),
        "live response should contain content or reasoning_content"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, DEEPSEEK_API_KEY, and network access"]
async fn live_chat_completion_stream_smoke_test() -> Result<(), Box<dyn Error>> {
    let Some(adapter) = live_adapter()? else {
        return Ok(());
    };

    let request = adapter
        .new_chat_request(vec![ChatMessage::user(
            "Reply with one short sentence confirming that streaming is reachable.",
        )])?
        .with_thinking(ThinkingConfig::disabled())
        .with_max_tokens(64);

    let mut stream = adapter.create_chat_completion_stream(request).await?;
    let mut saw_chunk = false;
    let mut saw_done = false;
    let mut saw_text = false;
    let mut saw_usage = false;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::Chunk(chunk) => {
                saw_chunk = true;
                saw_usage |= chunk.usage.is_some();
                saw_text |= chunk.choices.iter().any(|choice| {
                    choice
                        .delta
                        .content
                        .as_deref()
                        .is_some_and(|content| !content.trim().is_empty())
                        || choice
                            .delta
                            .reasoning_content
                            .as_deref()
                            .is_some_and(|content| !content.trim().is_empty())
                });
            }
            StreamEvent::Done => {
                saw_done = true;
                break;
            }
        }
    }

    assert!(saw_chunk, "live stream should produce at least one chunk");
    assert!(saw_done, "live stream should terminate with [DONE]");
    assert!(
        saw_text || saw_usage,
        "live stream should produce text/reasoning deltas or a usage chunk"
    );

    Ok(())
}
