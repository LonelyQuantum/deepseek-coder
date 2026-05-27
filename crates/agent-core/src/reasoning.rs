use crate::provider::deepseek_api::{ChatMessage, ChatRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningContentMode {
    ThinkingEnabled,
    ThinkingDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningContentState {
    NoReplayRequired,
    ReplayRequired { assistant_messages: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedReasoningMessages {
    pub messages: Vec<ChatMessage>,
    pub state: ReasoningContentState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReasoningContentError {
    #[error(
        "reasoning_content is only valid on assistant messages, got `{role:?}` at message {message_index}"
    )]
    UnexpectedReasoningContentRole {
        message_index: usize,
        role: ChatRole,
    },
    #[error("assistant message {message_index} has tool_calls but missing reasoning_content")]
    MissingRequiredReasoningContent { message_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReasoningContentStateMachine {
    mode: ReasoningContentMode,
}

impl ReasoningContentStateMachine {
    pub const fn thinking_enabled() -> Self {
        Self {
            mode: ReasoningContentMode::ThinkingEnabled,
        }
    }

    pub const fn thinking_disabled() -> Self {
        Self {
            mode: ReasoningContentMode::ThinkingDisabled,
        }
    }

    pub const fn mode(&self) -> ReasoningContentMode {
        self.mode
    }

    pub fn prepare_messages(
        &self,
        messages: &[ChatMessage],
    ) -> Result<PreparedReasoningMessages, ReasoningContentError> {
        let mut prepared = Vec::with_capacity(messages.len());
        let mut replay_required = 0;

        for (message_index, message) in messages.iter().cloned().enumerate() {
            if message.role != ChatRole::Assistant && message.reasoning_content.is_some() {
                return Err(ReasoningContentError::UnexpectedReasoningContentRole {
                    message_index,
                    role: message.role,
                });
            }

            let has_tool_calls = message
                .tool_calls
                .as_ref()
                .is_some_and(|tool_calls| !tool_calls.is_empty());

            let mut message = message;
            match (self.mode, message.role, has_tool_calls) {
                (ReasoningContentMode::ThinkingEnabled, ChatRole::Assistant, true) => {
                    if message
                        .reasoning_content
                        .as_deref()
                        .is_none_or(|reasoning_content| reasoning_content.trim().is_empty())
                    {
                        return Err(ReasoningContentError::MissingRequiredReasoningContent {
                            message_index,
                        });
                    }
                    replay_required += 1;
                }
                (ReasoningContentMode::ThinkingEnabled, ChatRole::Assistant, false)
                | (ReasoningContentMode::ThinkingDisabled, ChatRole::Assistant, _) => {
                    message.reasoning_content = None;
                }
                (_, _, _) => {}
            }

            prepared.push(message);
        }

        let state = if replay_required == 0 {
            ReasoningContentState::NoReplayRequired
        } else {
            ReasoningContentState::ReplayRequired {
                assistant_messages: replay_required,
            }
        };

        Ok(PreparedReasoningMessages {
            messages: prepared,
            state,
        })
    }
}

impl Default for ReasoningContentStateMachine {
    fn default() -> Self {
        Self::thinking_enabled()
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::deepseek_api::{ChatMessage, ChatRole, ChatToolCall};

    use super::{ReasoningContentError, ReasoningContentState, ReasoningContentStateMachine};

    fn assistant_with_reasoning(
        content: impl Into<String>,
        reasoning_content: impl Into<String>,
    ) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: Some(content.into()),
            name: None,
            prefix: None,
            reasoning_content: Some(reasoning_content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn tool_call() -> ChatToolCall {
        ChatToolCall::function("call_1", "read_file", r#"{"path":"README.md"}"#)
    }

    #[test]
    fn strips_reasoning_content_without_tool_calls() {
        let messages = vec![
            ChatMessage::user("hello"),
            assistant_with_reasoning("answer", "internal reasoning"),
            ChatMessage::user("next question"),
        ];

        let prepared = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect("messages should prepare");

        assert_eq!(prepared.state, ReasoningContentState::NoReplayRequired);
        assert_eq!(prepared.messages[1].reasoning_content, None);
    }

    #[test]
    fn preserves_reasoning_content_for_tool_call_assistant_messages() {
        let messages = vec![
            ChatMessage::user("read the README"),
            ChatMessage::assistant_with_tool_calls(
                Some("I need the file.".to_owned()),
                Some("The user asked about a file, so I should read it.".to_owned()),
                vec![tool_call()],
            ),
            ChatMessage::tool_result("call_1", "# prole-coder"),
            ChatMessage::user("continue"),
        ];

        let prepared = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect("messages should prepare");

        assert_eq!(
            prepared.state,
            ReasoningContentState::ReplayRequired {
                assistant_messages: 1
            }
        );
        assert_eq!(
            prepared.messages[1].reasoning_content.as_deref(),
            Some("The user asked about a file, so I should read it.")
        );
    }

    #[test]
    fn prepares_empty_message_list_without_replay_requirement() {
        let prepared = ReasoningContentStateMachine::default()
            .prepare_messages(&[])
            .expect("empty message list should prepare");

        assert!(prepared.messages.is_empty());
        assert_eq!(prepared.state, ReasoningContentState::NoReplayRequired);
    }

    #[test]
    fn counts_multiple_tool_call_assistant_messages_requiring_replay() {
        let messages = vec![
            ChatMessage::user("read two files"),
            ChatMessage::assistant_with_tool_calls(
                Some("Reading first.".to_owned()),
                Some("Need the first file.".to_owned()),
                vec![tool_call()],
            ),
            ChatMessage::tool_result("call_1", "first"),
            ChatMessage::assistant_with_tool_calls(
                Some("Reading second.".to_owned()),
                Some("Need the second file.".to_owned()),
                vec![ChatToolCall::function(
                    "call_2",
                    "read_file",
                    r#"{"path":"CHANGELOG.md"}"#,
                )],
            ),
            ChatMessage::tool_result("call_2", "second"),
        ];

        let prepared = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect("messages should prepare");

        assert_eq!(
            prepared.state,
            ReasoningContentState::ReplayRequired {
                assistant_messages: 2
            }
        );
        assert_eq!(
            prepared.messages[1].reasoning_content.as_deref(),
            Some("Need the first file.")
        );
        assert_eq!(
            prepared.messages[3].reasoning_content.as_deref(),
            Some("Need the second file.")
        );
    }

    #[test]
    fn keeps_only_tool_call_reasoning_across_later_user_turns() {
        let messages = vec![
            ChatMessage::user("read the README"),
            ChatMessage::assistant_with_tool_calls(
                None,
                Some("I need the README before answering.".to_owned()),
                vec![tool_call()],
            ),
            ChatMessage::tool_result("call_1", "# prole-coder"),
            assistant_with_reasoning("done", "I can now answer normally."),
            ChatMessage::user("ask another question"),
        ];

        let prepared = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect("messages should prepare");

        assert_eq!(
            prepared.messages[1].reasoning_content.as_deref(),
            Some("I need the README before answering.")
        );
        assert_eq!(prepared.messages[3].reasoning_content, None);
    }

    #[test]
    fn rejects_missing_tool_call_reasoning_when_thinking_is_enabled() {
        let messages = vec![
            ChatMessage::user("read the README"),
            ChatMessage::assistant_with_tool_calls(None, None, vec![tool_call()]),
        ];

        let error = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect_err("missing reasoning_content must fail");

        assert_eq!(
            error,
            ReasoningContentError::MissingRequiredReasoningContent { message_index: 1 }
        );
    }

    #[test]
    fn thinking_disabled_strips_tool_call_reasoning_without_replay_requirement() {
        let messages = vec![
            ChatMessage::user("read the README"),
            ChatMessage::assistant_with_tool_calls(
                None,
                Some("reasoning from an older request".to_owned()),
                vec![tool_call()],
            ),
        ];

        let prepared = ReasoningContentStateMachine::thinking_disabled()
            .prepare_messages(&messages)
            .expect("thinking-disabled mode should not require replay");

        assert_eq!(prepared.state, ReasoningContentState::NoReplayRequired);
        assert_eq!(prepared.messages[1].reasoning_content, None);
    }

    #[test]
    fn rejects_reasoning_content_on_non_assistant_messages() {
        let messages = vec![ChatMessage {
            role: ChatRole::User,
            content: Some("hello".to_owned()),
            name: None,
            prefix: None,
            reasoning_content: Some("invalid".to_owned()),
            tool_calls: None,
            tool_call_id: None,
        }];

        let error = ReasoningContentStateMachine::default()
            .prepare_messages(&messages)
            .expect_err("non-assistant reasoning_content must fail");

        assert_eq!(
            error,
            ReasoningContentError::UnexpectedReasoningContentRole {
                message_index: 0,
                role: ChatRole::User,
            }
        );
    }
}
