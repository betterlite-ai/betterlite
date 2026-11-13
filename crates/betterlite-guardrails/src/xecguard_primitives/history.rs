use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailInputType;

use super::message::{assistant_text_from_response, normalize_message, synthesize_user_message};
use super::wire::XecGuardMessage;

#[must_use]
pub fn build_full_history(
    request_data: &BTreeMap<String, GuardrailValue>,
    texts: &[String],
    input_type: GuardrailInputType,
) -> Vec<XecGuardMessage> {
    let mut messages = match request_data.get("messages") {
        Some(GuardrailValue::Array(items)) => items.iter().filter_map(normalize_message).collect(),
        _ => Vec::new(),
    };
    if input_type == GuardrailInputType::Request {
        if messages.is_empty() {
            return Vec::new();
        }
        if messages
            .last()
            .is_some_and(|message| message.role != "user")
        {
            let Some(synthesized) = synthesize_user_message(texts) else {
                return Vec::new();
            };
            messages.push(synthesized);
        }
        return messages;
    }
    let Some(assistant_text) = request_data
        .get("response")
        .and_then(assistant_text_from_response)
    else {
        return Vec::new();
    };
    messages.push(XecGuardMessage {
        role: "assistant".to_owned(),
        content: assistant_text,
    });
    messages
}

mod tests;
