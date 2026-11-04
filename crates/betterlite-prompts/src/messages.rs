use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};

const CONTINUE_TEXT: &str = "Please continue.";
pub const MAX_COMPLETION_MESSAGES: usize = 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MessageNormalizationError {
    #[error("messages exceeds the maximum of {limit}")]
    InputTooLong { limit: usize },
    #[error("normalized messages exceeds the maximum of {limit}")]
    OutputTooLong { limit: usize },
}

// System messages do not establish a turn role.
#[must_use]
pub fn detect_first_expected_role(messages: &[Message]) -> Option<&'static str> {
    for message in messages {
        match message.role.as_str() {
            "system" => {}
            "assistant" => return Some("user"),
            _ => return Some("assistant"),
        }
    }
    messages.is_empty().then_some("user")
}

fn counts_for_alternation(message: &Message) -> bool {
    if message.role == "user" {
        return true;
    }
    if message.role == "assistant" {
        return !message
            .provider_specific_fields
            .get("tool_calls")
            .is_some_and(json_list_is_nonempty);
    }
    false
}

const fn check_limit(
    messages: &[Message],
    too_long: MessageNormalizationError,
) -> Result<(), MessageNormalizationError> {
    if messages.len() > MAX_COMPLETION_MESSAGES {
        return Err(too_long);
    }
    Ok(())
}

fn continuation_placeholder(custom: Option<&Message>, role: &str) -> Message {
    custom.cloned().unwrap_or_else(|| Message {
        role: role.to_owned(),
        content: Some(Content::Text(CONTINUE_TEXT.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    })
}

const fn json_list_is_nonempty(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::List(items) => !items.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => true,
    }
}

// Clone before insertion so provider transforms never mutate shared request input.
pub fn insert_user_continue_message(
    messages: &[Message],
    custom: Option<&Message>,
    ensure: bool,
) -> Result<Vec<Message>, MessageNormalizationError> {
    let limit = MAX_COMPLETION_MESSAGES;
    check_limit(messages, MessageNormalizationError::InputTooLong { limit })?;
    let placeholder = continuation_placeholder(custom, "user");
    let mut result = Vec::with_capacity(messages.len().saturating_mul(2).saturating_add(1));
    let mut previous_counted_role = None;
    let mut previous_role = None;
    for (index, message) in messages.iter().enumerate() {
        let first_assistant = index == 0 && message.role == "assistant";
        if first_assistant
            || assistant_needs_user(
                message,
                previous_counted_role,
                previous_role == Some("assistant"),
            )
        {
            result.push(placeholder.clone());
        }
        result.push(message.clone());
        if counts_for_alternation(message) {
            previous_counted_role = Some(message.role.as_str());
        }
        previous_role = Some(message.role.as_str());
    }
    if ensure && previous_role == Some("assistant") {
        result.push(placeholder);
    }
    check_limit(&result, MessageNormalizationError::OutputTooLong { limit })?;
    Ok(result)
}

fn assistant_needs_user(
    message: &Message,
    previous_counted_role: Option<&str>,
    directly_after_assistant: bool,
) -> bool {
    counts_for_alternation(message)
        && message.role == "assistant"
        && (directly_after_assistant || previous_counted_role == Some("assistant"))
}

// Assistant placeholders only belong between adjacent counted user turns.
pub fn insert_assistant_continuation(
    messages: &[Message],
    custom: Option<&Message>,
    ensure: bool,
) -> Result<Vec<Message>, MessageNormalizationError> {
    let limit = MAX_COMPLETION_MESSAGES;
    check_limit(messages, MessageNormalizationError::InputTooLong { limit })?;
    let result = if ensure && messages.len() > 1 {
        let placeholder = continuation_placeholder(custom, "assistant");
        let mut previous_counted_user = false;
        let mut inserted = Vec::with_capacity(messages.len().saturating_mul(2));
        for message in messages {
            let counted_user = counts_for_alternation(message) && message.role == "user";
            if counted_user && previous_counted_user {
                inserted.push(placeholder.clone());
            }
            inserted.push(message.clone());
            if counts_for_alternation(message) {
                previous_counted_user = message.role == "user";
            }
        }
        inserted
    } else {
        messages.to_vec()
    };
    check_limit(&result, MessageNormalizationError::OutputTooLong { limit })?;
    Ok(result)
}

// Compose user and assistant insertion in LiteLLM's specified order.
pub fn get_completion_messages(
    messages: &[Message],
    assistant: Option<&Message>,
    user: Option<&Message>,
    ensure: bool,
) -> Result<Vec<Message>, MessageNormalizationError> {
    if !ensure {
        check_limit(
            messages,
            MessageNormalizationError::InputTooLong {
                limit: MAX_COMPLETION_MESSAGES,
            },
        )?;
        return Ok(messages.to_vec());
    }
    let with_users = insert_user_continue_message(messages, user, ensure)?;
    insert_assistant_continuation(&with_users, assistant, ensure)
}

mod tests;
