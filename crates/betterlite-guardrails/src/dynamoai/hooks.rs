use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::decisions::GuardrailStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHook {
    PreCall,
    PostCall,
    DuringCall,
}

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PreCall,
    EventHook::PostCall,
    EventHook::DuringCall,
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestData {
    pub messages: Vec<GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ResponseChoice {
    pub message: ResponseMessage,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ResponseMessage {
    pub role: Option<String>,
    pub content: Option<GuardrailValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostCallOutcome {
    NoText,
    Inspected(GuardrailStatus),
}

pub(super) fn extracted_message(choice: &ResponseChoice) -> Option<GuardrailValue> {
    let content = match choice.message.content.as_ref()? {
        GuardrailValue::Text(text) if !text.is_empty() => text.clone(),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => return None,
    };
    let role = choice
        .message
        .role
        .clone()
        .filter(|role| !role.is_empty())
        .unwrap_or_else(|| "assistant".to_owned());
    Some(GuardrailValue::Object(BTreeMap::from([
        ("content".to_owned(), GuardrailValue::Text(content)),
        ("role".to_owned(), GuardrailValue::Text(role)),
    ])))
}
