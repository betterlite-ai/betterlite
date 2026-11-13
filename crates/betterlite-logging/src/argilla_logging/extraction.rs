use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::settings::PayloadField;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct StandardLoggingPayload {
    #[serde(default)]
    pub messages: Option<Json>,
    #[serde(default)]
    pub response: Option<Json>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArgillaItem {
    pub fields: IndexMap<String, Json>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArgillaPayload {
    pub items: Vec<ArgillaItem>,
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("Error logging request payload. Payload=none.")]
    MissingPayload,
    #[error("No chat messages found in payload.")]
    MissingChatMessages,
    #[error("Invalid chat messages format: {value}")]
    InvalidChatMessages { value: String },
    #[error("No response found in payload.")]
    MissingResponse,
    #[error("Invalid response format: {value}")]
    InvalidResponse { value: String },
    #[error("the response has no choice carrying a message")]
    ResponseShape,
}

pub fn chat_messages(payload: &StandardLoggingPayload) -> Result<Vec<Json>, PayloadError> {
    let Some(messages) = payload.messages.as_ref() else {
        return Err(PayloadError::MissingChatMessages);
    };
    if let Json::Object(_) = messages {
        return Ok(vec![messages.clone()]);
    }
    if let Json::List(items) = messages
        && items
            .first()
            .is_some_and(|first| matches!(first, Json::Object(_)))
    {
        return Ok(items.clone());
    }
    Err(PayloadError::InvalidChatMessages {
        value: serde_json::to_string(messages).unwrap_or_default(),
    })
}

pub fn response_value(payload: &StandardLoggingPayload) -> Result<Json, PayloadError> {
    let Some(response) = payload.response.as_ref() else {
        return Err(PayloadError::MissingResponse);
    };
    match response {
        Json::Text(_) => Ok(response.clone()),
        Json::Object(fields) => first_choice_content(fields),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(PayloadError::InvalidResponse {
                value: serde_json::to_string(other).unwrap_or_default(),
            })
        }
    }
}

fn first_choice_content(fields: &IndexMap<String, Json>) -> Result<Json, PayloadError> {
    let choices = match fields.get("choices") {
        None => return Ok(Json::Text(String::new())),
        Some(Json::List(choices)) => choices,
        Some(_) => return Err(PayloadError::ResponseShape),
    };
    let Some(Json::Object(choice)) = choices.first() else {
        return Err(PayloadError::ResponseShape);
    };
    let Some(message) = choice.get("message") else {
        return Ok(Json::Text(String::new()));
    };
    let Json::Object(message_fields) = message else {
        return Err(PayloadError::ResponseShape);
    };
    message_fields.get("content").map_or_else(
        || Ok(Json::Text(String::new())),
        |content| Ok(content.clone()),
    )
}

pub fn prepare_log_data(
    transformation: &IndexMap<String, PayloadField>,
    kwargs: &CallbackKwargs,
) -> Result<ArgillaItem, PayloadError> {
    let Some(payload) = kwargs.standard_logging_object.as_ref() else {
        return Err(PayloadError::MissingPayload);
    };
    let messages = chat_messages(payload)?;
    let response = response_value(payload)?;
    let mut fields = IndexMap::with_capacity(transformation.len());
    for (name, field) in transformation {
        let value = match field {
            PayloadField::Messages => Json::List(messages.clone()),
            PayloadField::Response => response.clone(),
        };
        fields.insert(name.clone(), value);
    }
    Ok(ArgillaItem { fields })
}
