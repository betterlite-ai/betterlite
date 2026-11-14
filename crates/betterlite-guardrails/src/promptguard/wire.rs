use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::error::PromptGuardError;
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::inputs::GuardrailApiInputs;

pub const GUARD_ENDPOINT: &str = "/api/v1/guard";
const DEFAULT_THREAT_TYPE: &str = "unknown";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
}

impl From<GuardrailInputType> for Direction {
    fn from(input_type: GuardrailInputType) -> Self {
        match input_type {
            GuardrailInputType::Request => Self::Input,
            GuardrailInputType::Response => Self::Output,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GuardRequest {
    pub messages: Vec<GuardrailMessage>,
    pub direction: Direction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct GuardResponse {
    pub decision: Option<String>,
    pub threat_type: Option<String>,
    pub event_id: Option<String>,
    pub confidence: Option<f64>,
    pub redacted_messages: Option<Vec<GuardrailMessage>>,
}

#[must_use]
pub fn build_request(
    inputs: &GuardrailApiInputs,
    input_type: GuardrailInputType,
) -> Option<GuardRequest> {
    let messages = messages_for(inputs)?;
    Some(GuardRequest {
        messages,
        direction: Direction::from(input_type),
        model: inputs.model.clone(),
        images: inputs.images.clone().filter(|images| !images.is_empty()),
    })
}

fn messages_for(inputs: &GuardrailApiInputs) -> Option<Vec<GuardrailMessage>> {
    if let Some(messages) = inputs.structured_messages.as_ref()
        && !messages.is_empty()
    {
        return Some(messages.clone());
    }
    let texts = inputs.texts.as_deref().unwrap_or_default();
    if texts.is_empty() {
        return None;
    }
    Some(texts.iter().map(|text| user_message(text)).collect())
}

fn user_message(text: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([("content".to_owned(), GuardrailValue::Text(text.to_owned()))]),
    }
}

pub fn apply_decision(
    mut inputs: GuardrailApiInputs,
    response: GuardResponse,
) -> Result<GuardrailApiInputs, PromptGuardError> {
    match response.decision.as_deref() {
        Some("block") => Err(PromptGuardError::Blocked {
            threat_type: response
                .threat_type
                .as_deref()
                .unwrap_or(DEFAULT_THREAT_TYPE)
                .to_owned(),
            event_id: response.event_id.unwrap_or_default(),
            confidence: response.confidence.unwrap_or_default(),
        }),
        Some("redact") => {
            if let Some(redacted) = response
                .redacted_messages
                .as_deref()
                .filter(|messages| !messages.is_empty())
            {
                apply_redaction(&mut inputs, redacted);
            }
            Ok(inputs)
        }
        _ => Ok(inputs),
    }
}

fn apply_redaction(inputs: &mut GuardrailApiInputs, redacted: &[GuardrailMessage]) {
    if inputs
        .structured_messages
        .as_ref()
        .is_some_and(|messages| !messages.is_empty())
    {
        inputs.structured_messages = Some(redacted.to_vec());
    }
    if inputs.texts.is_some() {
        let extracted = extract_user_texts(redacted);
        if !extracted.is_empty() {
            inputs.texts = Some(extracted);
        }
    }
}

#[must_use]
pub fn extract_user_texts(messages: &[GuardrailMessage]) -> Vec<String> {
    let mut texts = Vec::new();
    for message in messages {
        if message.role == "user" {
            append_content_texts(message.extra.get("content"), &mut texts);
        }
    }
    texts
}

fn append_content_texts(content: Option<&GuardrailValue>, texts: &mut Vec<String>) {
    let Some(content) = content else {
        return;
    };
    if let GuardrailValue::Text(text) = content {
        texts.push(text.clone());
    }
    if let GuardrailValue::Array(parts) = content {
        for part in parts {
            append_part_text(part, texts);
        }
    }
}

fn append_part_text(part: &GuardrailValue, texts: &mut Vec<String>) {
    let GuardrailValue::Object(fields) = part else {
        return;
    };
    let is_text =
        matches!(fields.get("type"), Some(GuardrailValue::Text(kind)) if kind.as_str() == "text");
    if is_text
        && let Some(GuardrailValue::Text(text)) = fields.get("text")
        && !text.is_empty()
    {
        texts.push(text.clone());
    }
}
