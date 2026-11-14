use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::wire::RedactedMessage;
use crate::content_inspection::{InspectionMessage, build_inspection_messages};

const SAFE_ROLES: [&str; 3] = ["system", "user", "assistant"];

// AIM validates against the OpenAI chat schema, whose bare tool and function roles need fields the flatten drops
#[must_use]
pub fn projected_inspection_messages(
    data: &BTreeMap<String, GuardrailValue>,
) -> Vec<InspectionMessage> {
    build_inspection_messages(data)
        .into_iter()
        .map(|message| {
            if SAFE_ROLES.contains(&message.role.as_str()) {
                message
            } else {
                InspectionMessage {
                    role: "user".to_owned(),
                    content: message.content,
                }
            }
        })
        .collect()
}

fn text_field<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    name: &str,
) -> Option<&'data str> {
    match data.get(name) {
        Some(GuardrailValue::Text(value)) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[must_use]
pub fn user_email(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return None;
    };
    let Some(GuardrailValue::Object(headers)) = metadata.get("headers") else {
        return None;
    };
    text_field(headers, "x-aim-user-email")
}

#[must_use]
pub fn call_id(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    text_field(data, "litellm_call_id")
}

#[must_use]
pub fn redacted_value(message: &RedactedMessage) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "role".to_owned(),
            GuardrailValue::Text(message.role.clone()),
        ),
        (
            "content".to_owned(),
            GuardrailValue::Text(message.content.clone()),
        ),
    ]))
}
