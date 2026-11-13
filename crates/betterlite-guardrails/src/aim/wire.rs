use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::error::{AimError, BAD_REQUEST_CODE};
use crate::content_inspection::InspectionMessage;

#[derive(Debug, Serialize)]
pub struct AnalyzeRequest<'messages> {
    pub messages: &'messages [InspectionMessage],
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnalyzeResponse {
    pub required_action: Option<RequiredAction>,
    pub redacted_chat: Option<RedactedChat>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RequiredAction {
    pub action_type: Option<String>,
    pub detection_message: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RedactedChat {
    pub all_redacted_messages: Option<GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RedactedMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimAction {
    None,
    Monitor,
    Block,
    Anonymize,
    Unknown,
}

#[must_use]
pub fn action_of(response: &AnalyzeResponse) -> AimAction {
    let Some(action_type) = response
        .required_action
        .as_ref()
        .and_then(|action| action.action_type.as_deref())
    else {
        return AimAction::None;
    };
    match action_type {
        "monitor_action" => AimAction::Monitor,
        "block_action" => AimAction::Block,
        "anonymize_action" => AimAction::Anonymize,
        _ => AimAction::Unknown,
    }
}

#[must_use]
pub fn malformed_redaction(message: &str) -> AimError {
    AimError::Rejected {
        message: message.to_owned(),
        code: BAD_REQUEST_CODE,
    }
}

pub fn redacted_messages(chat: &RedactedChat) -> Result<Vec<RedactedMessage>, AimError> {
    let Some(value) = chat.all_redacted_messages.as_ref() else {
        return Err(malformed_redaction(
            "Aim returned no redacted messages, so the content can not be rewritten",
        ));
    };
    let json = serde_json::to_value(value).map_err(|_error| {
        malformed_redaction("Aim returned redacted messages that are not JSON")
    })?;
    serde_json::from_value(json).map_err(|_error| {
        malformed_redaction(
            "Aim returned malformed redacted messages, so the content can not be rewritten",
        )
    })
}
