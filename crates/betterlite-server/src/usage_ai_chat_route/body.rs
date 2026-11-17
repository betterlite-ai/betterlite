pub mod issue;

mod response;

use std::str::Utf8Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use thiserror::Error;

use crate::usage_ai_chat::messages::ChatMessage;

use self::issue::{IssueKind, LocPart, ValidationIssue};

const BODY: &str = "body";
const MESSAGES: &str = "messages";
const MODEL: &str = "model";
const ROLE: &str = "role";
const CONTENT: &str = "content";
const ROLE_EXPECTED: &str = "'user' or 'assistant'";
const ROLE_KIND: IssueKind = IssueKind::NotOneOf {
    expected: ROLE_EXPECTED,
};

pub struct ChatBody {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) model: Option<String>,
}

#[derive(Debug, Error)]
pub enum BodyError {
    #[error("the request body is not valid UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the request body is not valid JSON")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request body does not match the declared types")]
    Issues { issues: Vec<ValidationIssue> },
}

enum ChatRole {
    User,
    Assistant,
}

pub fn decode_body(body: &[u8]) -> Result<ChatBody, BodyError> {
    let text = std::str::from_utf8(body).map_err(|source| BodyError::NotUtf8 { source })?;
    if text.is_empty() {
        return Err(BodyError::Issues {
            issues: vec![ValidationIssue::of(
                IssueKind::Missing,
                vec![LocPart::Field(BODY)],
                JsonValue::Null,
            )],
        });
    }
    let value: JsonValue =
        serde_json::from_str(text).map_err(|source| BodyError::InvalidJson { source })?;
    decode_value(&value)
}

fn decode_value(value: &JsonValue) -> Result<ChatBody, BodyError> {
    let JsonValue::Object(fields) = value else {
        return Err(BodyError::Issues {
            issues: vec![ValidationIssue::of(
                IssueKind::NotObject,
                vec![LocPart::Field(BODY)],
                value.clone(),
            )],
        });
    };
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let messages = decode_messages(fields, value, &mut issues);
    let model = decode_model(fields, &mut issues);
    if issues.is_empty() {
        return Ok(ChatBody { messages, model });
    }
    Err(BodyError::Issues { issues })
}

fn decode_messages(
    fields: &JsonMap,
    value: &JsonValue,
    issues: &mut Vec<ValidationIssue>,
) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    match fields.get(MESSAGES) {
        Some(JsonValue::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let JsonValue::Object(message) = item else {
                    issues.push(ValidationIssue::of(
                        IssueKind::NotObject,
                        vec![
                            LocPart::Field(BODY),
                            LocPart::Field(MESSAGES),
                            LocPart::Index(index),
                        ],
                        item.clone(),
                    ));
                    continue;
                };
                if let Some(chat) = decode_message(message, index, issues) {
                    messages.push(chat);
                }
            }
        }
        Some(other) => issues.push(ValidationIssue::of(
            IssueKind::NotList,
            vec![LocPart::Field(BODY), LocPart::Field(MESSAGES)],
            other.clone(),
        )),
        None => issues.push(ValidationIssue::of(
            IssueKind::Missing,
            vec![LocPart::Field(BODY), LocPart::Field(MESSAGES)],
            value.clone(),
        )),
    }
    messages
}

fn decode_model(fields: &JsonMap, issues: &mut Vec<ValidationIssue>) -> Option<String> {
    match fields.get(MODEL) {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(name)) => Some(name.clone()),
        Some(other) => {
            issues.push(ValidationIssue::of(
                IssueKind::NotText,
                vec![LocPart::Field(BODY), LocPart::Field(MODEL)],
                other.clone(),
            ));
            None
        }
    }
}

fn field(
    message: &JsonMap,
    name: &'static str,
    index: usize,
    issues: &mut Vec<ValidationIssue>,
) -> Option<JsonValue> {
    let Some(value) = message.get(name) else {
        issues.push(ValidationIssue::at_field(
            IssueKind::Missing,
            index,
            name,
            JsonValue::Object(message.clone()),
        ));
        return None;
    };
    Some(value.clone())
}

fn decode_message(
    message: &JsonMap,
    index: usize,
    issues: &mut Vec<ValidationIssue>,
) -> Option<ChatMessage> {
    let parsed_role = match field(message, ROLE, index, issues) {
        Some(JsonValue::String(name)) if name == "user" => Some(ChatRole::User),
        Some(JsonValue::String(name)) if name == "assistant" => Some(ChatRole::Assistant),
        Some(other) => {
            issues.push(ValidationIssue::at_field(ROLE_KIND, index, ROLE, other));
            None
        }
        None => None,
    };
    let parsed_content = match field(message, CONTENT, index, issues) {
        Some(JsonValue::String(text)) => Some(text),
        Some(other) => {
            issues.push(ValidationIssue::at_field(
                IssueKind::NotText,
                index,
                CONTENT,
                other,
            ));
            None
        }
        None => None,
    };
    let (Some(role), Some(content)) = (parsed_role, parsed_content) else {
        return None;
    };
    Some(match role {
        ChatRole::User => ChatMessage::User { content },
        ChatRole::Assistant => ChatMessage::Assistant { content },
    })
}
