use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::wire::RedactedMessage;
use crate::content_inspection::{
    InspectionMessage, apply_redacted_messages_back, build_inspection_messages,
};

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

#[must_use]
pub fn user_email(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return None;
    };
    let Some(GuardrailValue::Object(headers)) = metadata.get("headers") else {
        return None;
    };
    match headers.get("x-aim-user-email") {
        Some(GuardrailValue::Text(email)) if !email.is_empty() => Some(email),
        _ => None,
    }
}

#[must_use]
pub fn call_id(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match data.get("litellm_call_id") {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id),
        _ => None,
    }
}

#[must_use]
pub fn apply_anonymize(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted: &[RedactedMessage],
) -> bool {
    let messages: Vec<GuardrailValue> = redacted
        .iter()
        .map(|message| {
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
        })
        .collect();
    apply_redacted_messages_back(data, &messages)
}
