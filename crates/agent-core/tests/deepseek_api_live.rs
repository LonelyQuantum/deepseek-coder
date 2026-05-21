use std::{env, error::Error, time::Duration};

use deepseek_coder_agent_core::provider::deepseek_api::{
    ChatMessage, DeepSeekApiAdapter, DeepSeekApiConfig, StreamEvent, ThinkingConfig,
};
use futures_util::StreamExt;

const LIVE_TEST_FLAG: &str = "DEEPSEEK_CODER_LIVE_TESTS";

fn live_adapter() -> Result<Option<DeepSeekApiAdapter>, Box<dyn Error>> {
    if env::var(LIVE_TEST_FLAG).ok().as_deref() != Some("1") {
        eprintln!("skipping live DeepSeek API test: set {LIVE_TEST_FLAG}=1 to enable");
        return Ok(None);
    }

    let config = DeepSeekApiConfig::from_env()?.with_timeout(Duration::from_secs(120));
    Ok(Some(DeepSeekApiAdapter::new(config)?))
}

#[tokio::test]
#[ignore = "requires DEEPSEEK_CODER_LIVE_TESTS=1, DEEPSEEK_API_KEY, and network access"]
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
