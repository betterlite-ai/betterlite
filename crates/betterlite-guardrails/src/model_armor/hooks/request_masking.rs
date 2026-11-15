use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::last_user::set_last_user_message;

use super::Data;
use super::error_handling::FieldError;
use super::masking::changed_text;

pub(super) const MESSAGES_KEY: &str = "messages";
const MESSAGES_EXPECTED: &str = "a list of messages with a string role";

// LiteLLM's `if not messages`: an absent, null or empty list leaves the hook with nothing to scan
pub(super) fn read_messages(data: &Data) -> Result<Option<Vec<Message>>, FieldError> {
    let items = match data.get(MESSAGES_KEY) {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(items)) => items,
        Some(_) => {
            return Err(FieldError {
                field: MESSAGES_KEY,
                expected: MESSAGES_EXPECTED,
            });
        }
    };
    let encoded = serde_json::to_value(items).map_err(|_encode| FieldError {
        field: MESSAGES_KEY,
        expected: MESSAGES_EXPECTED,
    })?;
    let messages: Vec<Message> = serde_json::from_value(encoded).map_err(|_decode| FieldError {
        field: MESSAGES_KEY,
        expected: MESSAGES_EXPECTED,
    })?;
    Ok((!messages.is_empty()).then_some(messages))
}

fn write_messages(data: &mut Data, messages: &[Message]) -> Result<(), FieldError> {
    let encoded = serde_json::to_value(messages).map_err(|_encode| FieldError {
        field: MESSAGES_KEY,
        expected: MESSAGES_EXPECTED,
    })?;
    let value: GuardrailValue = serde_json::from_value(encoded).map_err(|_decode| FieldError {
        field: MESSAGES_KEY,
        expected: MESSAGES_EXPECTED,
    })?;
    data.insert(MESSAGES_KEY.to_owned(), value);
    Ok(())
}

// LiteLLM's mask_request_content branch, applied only when Model Armor sanitized the scanned text
pub(super) fn apply(
    data: &mut Data,
    messages: &[Message],
    content: &str,
    sanitized: Option<String>,
) -> Result<(), FieldError> {
    let Some(sanitized) = changed_text(content, sanitized) else {
        return Ok(());
    };
    let masked = set_last_user_message(messages, &sanitized);
    write_messages(data, &masked)
}

mod tests;
