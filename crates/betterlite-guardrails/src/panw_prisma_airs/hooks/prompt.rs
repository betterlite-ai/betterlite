use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::data::{Data, MESSAGES, PROMPT, parse_items};
use super::error::HookDataError;
use crate::prisma_airs_extraction::{Message, extract_text_from_messages};

pub fn extract_prompt_from_request(data: &Data) -> Result<String, HookDataError> {
    let prompt_text = match data.get(MESSAGES) {
        Some(GuardrailValue::Array(messages)) => {
            let parsed: Vec<Message> = parse_items(messages, MESSAGES)?;
            extract_text_from_messages(&parsed)
        }
        None | Some(_) => String::new(),
    };
    if !prompt_text.is_empty() {
        return Ok(prompt_text);
    }
    match data.get(PROMPT) {
        Some(GuardrailValue::Text(text)) => Ok(text.clone()),
        Some(GuardrailValue::Array(items)) => join_prompt_items(items),
        None | Some(_) => Ok(String::new()),
    }
}

// A prompt list holds strings, token lists or string lists; LiteLLM joins every non-empty one, a list rendered as JSON text.
fn join_prompt_items(items: &[GuardrailValue]) -> Result<String, HookDataError> {
    let mut parts = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            GuardrailValue::Text(text) if !text.is_empty() => parts.push(text.clone()),
            GuardrailValue::Array(list) if list.is_empty() => {}
            GuardrailValue::Null | GuardrailValue::Text(_) => {}
            GuardrailValue::Boolean(_) | GuardrailValue::Object(_) => {
                return Err(HookDataError::NotText {
                    field: PROMPT,
                    index,
                });
            }
            GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_) => {
                let rendered =
                    serde_json::to_string(item).map_err(|source| HookDataError::Item {
                        field: PROMPT,
                        index,
                        source,
                    })?;
                parts.push(rendered);
            }
        }
    }
    Ok(parts.join(" "))
}
