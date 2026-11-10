use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::json;

// the content block types LiteLLM accepts in a user message
const USER_CONTENT_TYPES: [&str; 10] = [
    "text",
    "image_url",
    "input_audio",
    "audio_url",
    "document",
    "guarded_text",
    "grounding_source",
    "query",
    "video_url",
    "file",
];
const MAX_STOP_SEQUENCES: usize = 4;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error(
        "Invalid message at index {index}. Please ensure all messages are valid OpenAI chat completion messages."
    )]
    InvalidMessage { index: usize },
    #[error(
        "Invalid user message at index {index}. Please ensure all user messages are valid OpenAI chat completion messages."
    )]
    InvalidUserMessage { index: usize },
    #[error("the message is not a JSON object")]
    NotAnObject,
    #[error("the tool calls cannot be iterated")]
    ToolCallsNotIterable,
    #[error(
        "Invalid tool choice, tool_choice={tool_choice}. Please ensure tool_choice follows the OpenAI spec"
    )]
    InvalidToolChoice { tool_choice: String },
    #[error(
        "Invalid tool choice, tool_choice={tool_choice}. Got={kind}. Expecting str, or dict. Please ensure tool_choice follows the OpenAI tool_choice spec"
    )]
    InvalidToolChoiceType {
        tool_choice: String,
        kind: &'static str,
    },
}

impl ValidationError {
    // LiteLLM raises BadRequestError for a tool choice and a plain exception for everything else here
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        match self {
            Self::InvalidToolChoice { .. } | Self::InvalidToolChoiceType { .. } => {
                let context = ErrorContext {
                    message: self.to_string(),
                    model: None,
                    provider: Some(String::new()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                };
                Some(ErrorClass::BadRequest(Box::new(ResponseDetails::from(
                    context,
                ))))
            }
            Self::InvalidMessage { .. }
            | Self::InvalidUserMessage { .. }
            | Self::NotAnObject
            | Self::ToolCallsNotIterable => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopSequences {
    One(String),
    Many(Vec<String>),
}

pub fn convert_to_dict(message: &Json) -> Result<&IndexMap<String, Json>, ValidationError> {
    match json::object(message) {
        Some(fields) => Ok(fields),
        None => Err(ValidationError::NotAnObject),
    }
}

pub fn convert_list_message_to_dict(messages: &[Json]) -> Result<Vec<Json>, ValidationError> {
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        convert_to_dict(message)?;
        converted.push(cleanup_none_field_in_message(message)?);
    }
    Ok(converted)
}

pub fn cleanup_none_field_in_message(message: &Json) -> Result<Json, ValidationError> {
    let fields = convert_to_dict(message)?;
    let cleaned: IndexMap<String, Json> = fields
        .iter()
        .filter(|(_key, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(Json::Object(cleaned))
}

pub fn fix_openai_messages(messages: &[Json]) -> Result<Vec<Json>, ValidationError> {
    let mut new_messages = Vec::with_capacity(messages.len());
    for message in messages {
        let mut fields = convert_to_dict(message)?.clone();
        if !fields.get("role").is_some_and(json::counts_as_present) {
            fields.insert(String::from("role"), Json::Text(String::from("assistant")));
        }
        if let Some(found) = fields
            .get("tool_calls")
            .filter(|value| json::counts_as_present(value))
        {
            let calls = tool_calls_as_list(found)?;
            fields.insert(String::from("tool_calls"), Json::List(calls));
        }
        new_messages.push(cleanup_none_field_in_message(&Json::Object(fields))?);
    }
    validate_openai_user_messages(&new_messages)
}

fn tool_calls_as_list(tool_calls: &Json) -> Result<Vec<Json>, ValidationError> {
    match tool_calls {
        Json::List(items) => Ok(items
            .iter()
            .filter(|item| matches!(item, Json::Object(_)))
            .cloned()
            .collect()),
        // a string or an object iterates into names, and none of those is a dict
        Json::Text(_) | Json::Object(_) => Ok(Vec::new()),
        Json::Null | Json::Bool(_) | Json::Number(_) => Err(ValidationError::ToolCallsNotIterable),
    }
}

#[must_use]
pub fn validate_and_fix_openai_tools(tools: Option<&[Json]>) -> Option<Vec<Json>> {
    let found = tools?;
    Some(
        found
            .iter()
            .filter(|tool| matches!(tool, Json::Object(_)))
            .cloned()
            .collect(),
    )
}

#[must_use]
pub fn fix_thinking_param(thinking: &Json, medium_budget: u32) -> Option<Json> {
    match thinking {
        Json::Bool(true) => {
            let mut enabled = IndexMap::new();
            enabled.insert(String::from("type"), Json::Text(String::from("enabled")));
            enabled.insert(
                String::from("budget_tokens"),
                Json::Number(serde_json::Number::from(medium_budget)),
            );
            Some(Json::Object(enabled))
        }
        Json::Bool(false) | Json::Null => None,
        Json::Object(fields) => {
            let mut normalized = fields.clone();
            if let Some(budget) = normalized.shift_remove("budgetTokens")
                && !normalized.contains_key("budget_tokens")
            {
                normalized.insert(String::from("budget_tokens"), budget);
            }
            Some(Json::Object(normalized))
        }
        Json::Number(_) | Json::Text(_) | Json::List(_) => Some(thinking.clone()),
    }
}

pub fn validate_openai_user_messages(messages: &[Json]) -> Result<Vec<Json>, ValidationError> {
    for (index, message) in messages.iter().enumerate() {
        let Some(fields) = json::object(message) else {
            return Err(ValidationError::InvalidMessage { index });
        };
        let Some(role) = fields.get("role") else {
            return Err(ValidationError::InvalidMessage { index });
        };
        if json::text(role) != Some("user") {
            continue;
        }
        if let Some(Json::List(items)) = fields.get("content") {
            let valid = items.iter().filter_map(json::object).all(|item| {
                item.get("type")
                    .and_then(json::text)
                    .is_some_and(|kind| USER_CONTENT_TYPES.contains(&kind))
            });
            if !valid {
                return Err(ValidationError::InvalidUserMessage { index });
            }
        }
    }
    Ok(messages.to_vec())
}

pub fn validate_tool_choice(tool_choice: &Json) -> Result<Json, ValidationError> {
    match tool_choice {
        Json::Null | Json::Text(_) => Ok(tool_choice.clone()),
        Json::Object(fields) => {
            if let Some(Json::Text(kind)) = fields.get("type")
                && matches!(kind.as_str(), "auto" | "none" | "required")
                && !fields.contains_key("function")
            {
                return Ok(Json::Text(kind.clone()));
            }
            let type_absent = matches!(fields.get("type"), None | Some(Json::Null));
            let function_absent = matches!(fields.get("function"), None | Some(Json::Null));
            if type_absent || function_absent {
                return Err(ValidationError::InvalidToolChoice {
                    tool_choice: serde_json::to_string(tool_choice).unwrap_or_default(),
                });
            }
            Ok(tool_choice.clone())
        }
        Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(ValidationError::InvalidToolChoiceType {
                tool_choice: serde_json::to_string(tool_choice).unwrap_or_default(),
                kind: json_kind(tool_choice),
            })
        }
    }
}

const fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "list",
        Json::Object(_) => "object",
    }
}

#[must_use]
pub fn validate_optional_params(
    stop: Option<StopSequences>,
    disable_stop_sequence_limit: bool,
) -> Option<StopSequences> {
    match stop {
        Some(StopSequences::Many(mut sequences)) if !disable_stop_sequence_limit => {
            sequences.truncate(MAX_STOP_SEQUENCES);
            Some(StopSequences::Many(sequences))
        }
        Some(StopSequences::One(sequence)) => Some(StopSequences::One(sequence)),
        Some(StopSequences::Many(sequences)) => Some(StopSequences::Many(sequences)),
        None => None,
    }
}
