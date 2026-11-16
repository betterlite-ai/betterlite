use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::message_content::Message;

use super::Data;
use super::error_handling::FieldError;
use super::masking::changed_text;

pub(super) const MESSAGES_KEY: &str = "messages";
const MESSAGES_EXPECTED: &str = "a list of messages with a string role";
const USER_ROLE: &str = "user";

// LiteLLM's `if not messages`: an absent, null or empty list leaves the hook with nothing to scan
pub(super) fn read_messages(data: &Data) -> Result<Option<Vec<Message>>, FieldError> {
    let items = match data.get(MESSAGES_KEY) {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(items)) => items,
        Some(_) => {
            return Err(FieldError {
                field: MESSAGES_KEY,
                expected: MESSAGES_EXPECTED,
                source: None,
            });
        }
    };
    let messages: Vec<Message> = serde_json::to_value(items)
        .and_then(serde_json::from_value)
        .map_err(|source| FieldError {
            field: MESSAGES_KEY,
            expected: MESSAGES_EXPECTED,
            source: Some(source),
        })?;
    Ok((!messages.is_empty()).then_some(messages))
}

// LiteLLM's set_last_user_message: drop the trailing user run, append the masked turn, keep the rest
pub(super) fn apply(
    data: &mut Data,
    messages: &[Message],
    content: &str,
    sanitized: Option<String>,
) {
    let Some(text) = changed_text(content, sanitized) else {
        return;
    };
    let Some(GuardrailValue::Array(items)) = data.get_mut(MESSAGES_KEY) else {
        return;
    };
    let trailing_users = messages
        .iter()
        .rev()
        .take_while(|message| message.role == USER_ROLE)
        .count();
    items.truncate(items.len().saturating_sub(trailing_users));
    items.push(GuardrailValue::Object(BTreeMap::from([
        (
            "role".to_owned(),
            GuardrailValue::Text(USER_ROLE.to_owned()),
        ),
        ("content".to_owned(), GuardrailValue::Text(text)),
    ])));
}

mod tests;
