use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::decisions::{GuardrailStatus, create_error_message, determine_status, process_response};
use super::error::DynamoAiError;
use super::guardrail::DynamoAiGuardrail;

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
    pub messages: Option<Vec<GuardrailValue>>,
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

impl DynamoAiGuardrail {
    pub async fn async_pre_call_hook(
        &self,
        data: &RequestData,
    ) -> Result<GuardrailStatus, DynamoAiError> {
        match data.messages.as_deref() {
            Some(messages) if !messages.is_empty() => self.inspect_messages(messages).await,
            Some(_messages) | None => Ok(GuardrailStatus::Success),
        }
    }

    pub async fn async_moderation_hook(
        &self,
        data: &RequestData,
    ) -> Result<GuardrailStatus, DynamoAiError> {
        match data.messages.as_deref() {
            Some(messages) if !messages.is_empty() => self.inspect_messages(messages).await,
            Some(_messages) | None => Ok(GuardrailStatus::Success),
        }
    }

    pub async fn async_post_call_success_hook(
        &self,
        choices: &[ResponseChoice],
    ) -> Result<PostCallOutcome, DynamoAiError> {
        let messages: Vec<GuardrailValue> = choices.iter().filter_map(extracted_message).collect();
        if messages.is_empty() {
            return Ok(PostCallOutcome::NoText);
        }
        let status = self.inspect_messages(&messages).await?;
        Ok(PostCallOutcome::Inspected(status))
    }

    async fn inspect_messages(
        &self,
        messages: &[GuardrailValue],
    ) -> Result<GuardrailStatus, DynamoAiError> {
        let response = self.call_moderation(messages).await?;
        let status = determine_status(&response);
        let processed = process_response(&response);
        if processed.violations_detected.is_empty() {
            return Ok(status);
        }
        Err(DynamoAiError::Blocked {
            message: create_error_message(&processed),
        })
    }
}

fn extracted_message(choice: &ResponseChoice) -> Option<GuardrailValue> {
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
