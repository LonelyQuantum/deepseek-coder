use std::{env, error::Error, time::Duration};

use futures_util::StreamExt;
use prole_coder_agent_core::{
    provider::deepseek_api::{
        ChatFunctionDefinition, ChatMessage, ChatTool, ChatToolCallAccumulator,
        DEFAULT_API_BASE_URL, DEFAULT_MODEL, DeepSeekApiAdapter, DeepSeekApiConfig, StreamEvent,
        ThinkingConfig, ToolChoice, Usage,
    },
    reasoning::{ReasoningContentState, ReasoningContentStateMachine},
    test_helpers::{LIVE_TEST_FLAG, live_api_key, repo_root_from_crate_manifest},
};

fn live_config() -> Result<DeepSeekApiConfig, Box<dyn Error>> {
    let api_key = live_api_key(repo_root_from_crate_manifest(env!("CARGO_MANIFEST_DIR")))?;
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
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, and network access"]
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
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, and network access"]
async fn live_reasoning_content_tool_replay_smoke_test() -> Result<(), Box<dyn Error>> {
    let Some(adapter) = live_adapter()? else {
        return Ok(());
    };

    let system_message = ChatMessage::system(
        "For this live test, use the provided tool before giving the final answer.",
    );
    let user_message = ChatMessage::user(
        "Call get_live_reasoning_fixture exactly once. After the tool result, reply with the exact token OK_REASONING_REPLAY.",
    );
    let request = adapter
        .new_chat_request(vec![system_message.clone(), user_message.clone()])?
        .with_thinking(ThinkingConfig::enabled())
        .with_max_tokens(256)
        .with_tools(vec![live_reasoning_probe_tool()]);

    let response = adapter.create_chat_completion(request).await?;
    let choice = response
        .choices
        .first()
        .ok_or("live reasoning response must contain at least one choice")?;
    let reasoning_content = choice
        .message
        .reasoning_content
        .as_deref()
        .ok_or("live reasoning tool-call response must include reasoning_content")?;
    assert!(
        !reasoning_content.trim().is_empty(),
        "live reasoning_content should not be empty"
    );

    let tool_calls = choice
        .message
        .tool_calls
        .clone()
        .ok_or("live reasoning response must include a tool call")?;
    assert_eq!(
        tool_calls.len(),
        1,
        "live reasoning test expects one tool call"
    );
    assert_eq!(
        tool_calls[0].function.name, "get_live_reasoning_fixture",
        "live reasoning test should call the forced fixture tool"
    );

    let mut messages = vec![system_message, user_message];
    messages.push(ChatMessage::assistant_with_tool_calls(
        choice.message.content.clone(),
        choice.message.reasoning_content.clone(),
        tool_calls.clone(),
    ));
    messages.push(ChatMessage::tool_result(
        tool_calls[0].id.clone(),
        "fixture_token=OK_REASONING_REPLAY",
    ));

    let prepared = ReasoningContentStateMachine::thinking_enabled().prepare_messages(&messages)?;
    assert_eq!(
        prepared.state,
        ReasoningContentState::ReplayRequired {
            assistant_messages: 1
        }
    );
    assert_eq!(
        prepared.messages[2].reasoning_content.as_deref(),
        Some(reasoning_content),
        "state machine must preserve tool-call reasoning_content for replay"
    );

    let follow_up = adapter
        .new_chat_request(prepared.messages)?
        .with_thinking(ThinkingConfig::enabled())
        .with_max_tokens(128);
    let follow_up_response = adapter.create_chat_completion(follow_up).await?;
    let final_content = follow_up_response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .ok_or("live reasoning follow-up response must include content")?;

    assert!(
        final_content.contains("OK_REASONING_REPLAY"),
        "live reasoning follow-up should use the replayed tool result; got: {final_content}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, and network access"]
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

#[tokio::test]
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, and network access"]
async fn live_streaming_tool_call_accumulator_smoke_test() -> Result<(), Box<dyn Error>> {
    let Some(adapter) = live_adapter()? else {
        return Ok(());
    };

    let request = adapter
        .new_chat_request(vec![
            ChatMessage::system(
                "For this live test, call the selected tool exactly once and do not answer in prose.",
            ),
            ChatMessage::user("Call get_live_reasoning_fixture with no arguments."),
        ])?
        .with_thinking(ThinkingConfig::disabled())
        .with_max_tokens(128)
        .with_tools(vec![live_reasoning_probe_tool()])
        .with_tool_choice(ToolChoice::function("get_live_reasoning_fixture"));

    let mut stream = adapter.create_chat_completion_stream(request).await?;
    let mut accumulator = ChatToolCallAccumulator::new();
    let mut saw_chunk = false;
    let mut saw_done = false;
    let mut saw_tool_delta = false;
    let mut saw_usage = false;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::Chunk(chunk) => {
                saw_chunk = true;
                saw_usage |= chunk.usage.is_some();

                for choice in chunk.choices {
                    if let Some(tool_call_deltas) = choice.delta.tool_calls {
                        saw_tool_delta = true;
                        for tool_call_delta in tool_call_deltas {
                            accumulator.append_delta(tool_call_delta)?;
                        }
                    }
                }
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
        saw_tool_delta,
        "live stream should produce tool call deltas"
    );
    assert!(
        saw_usage,
        "live stream should produce the include_usage summary chunk"
    );

    let tool_calls = accumulator.finish()?;
    assert_eq!(
        tool_calls.len(),
        1,
        "live streaming tool test expects one tool call"
    );
    assert_eq!(
        tool_calls[0].function.name, "get_live_reasoning_fixture",
        "streaming accumulator should preserve the selected function name"
    );
    let arguments: serde_json::Value = serde_json::from_str(&tool_calls[0].function.arguments)?;
    assert!(
        arguments.is_object(),
        "tool call arguments should assemble into a JSON object"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires PROLE_CODER_LIVE_TESTS=1, API key, network access, and manual cache observation"]
async fn live_cache_usage_summary_smoke_test() -> Result<(), Box<dyn Error>> {
    let Some(adapter) = live_adapter()? else {
        return Ok(());
    };

    let stable_prefix = "Stable cache probe prefix for prole-coder. ".repeat(256);
    let first_usage = collect_stream_usage(
        &adapter,
        stable_prefix.clone(),
        "First cache usage probe. Reply with CACHE_PROBE_ONE.",
    )
    .await?
    .ok_or("first live cache probe should produce usage")?;
    let second_usage = collect_stream_usage(
        &adapter,
        stable_prefix,
        "Second cache usage probe with the same stable prefix. Reply with CACHE_PROBE_TWO.",
    )
    .await?
    .ok_or("second live cache probe should produce usage")?;

    eprintln!(
        "first cache usage: hit={:?}, miss={:?}, prompt={}",
        first_usage.prompt_cache_hit_tokens,
        first_usage.prompt_cache_miss_tokens,
        first_usage.prompt_tokens
    );
    eprintln!(
        "second cache usage: hit={:?}, miss={:?}, prompt={}",
        second_usage.prompt_cache_hit_tokens,
        second_usage.prompt_cache_miss_tokens,
        second_usage.prompt_tokens
    );

    assert!(
        second_usage.prompt_cache_hit_tokens.is_some()
            || second_usage.prompt_cache_miss_tokens.is_some(),
        "live cache usage probe should expose prompt cache hit/miss fields"
    );

    Ok(())
}

async fn collect_stream_usage(
    adapter: &DeepSeekApiAdapter,
    stable_prefix: String,
    user_task: &str,
) -> Result<Option<Usage>, Box<dyn Error>> {
    let request = adapter
        .new_chat_request(vec![
            ChatMessage::system(stable_prefix),
            ChatMessage::user(user_task),
        ])?
        .with_thinking(ThinkingConfig::disabled())
        .with_max_tokens(32);
    let mut stream = adapter.create_chat_completion_stream(request).await?;
    let mut usage = None;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::Chunk(chunk) => {
                if chunk.usage.is_some() {
                    usage = chunk.usage;
                }
            }
            StreamEvent::Done => break,
        }
    }

    Ok(usage)
}
