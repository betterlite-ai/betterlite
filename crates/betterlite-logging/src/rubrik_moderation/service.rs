use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik::block_decision::{
    ModerationChoice, ModerationMessage, ModerationServiceResponse, ReturnedToolCall,
};

use super::context::text_field;
use super::error::ModerationError;

const fn object_fields<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value IndexMap<String, Json>, ModerationError> {
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ModerationError::FieldType { field })
        }
    }
}

// Reads the webhook's OpenAI-shaped answer: only choices[0] matters, so a later malformed choice cannot fail a request over nothing.
pub fn service_response(
    fields: &IndexMap<String, Json>,
) -> Result<ModerationServiceResponse, ModerationError> {
    let choices = match fields.get("choices") {
        None | Some(Json::Null) => {
            return Ok(ModerationServiceResponse {
                choices: Vec::new(),
            });
        }
        Some(Json::List(choices)) => choices,
        Some(_) => return Err(ModerationError::FieldType { field: "choices" }),
    };
    Ok(ModerationServiceResponse {
        choices: choices
            .first()
            .map(moderation_choice)
            .transpose()?
            .into_iter()
            .collect(),
    })
}

fn moderation_choice(value: &Json) -> Result<ModerationChoice, ModerationError> {
    let fields = object_fields(value, "choices[0]")?;
    let message = match fields.get("message") {
        None | Some(Json::Null) => None,
        Some(Json::Object(message)) => Some(moderation_message(message)?),
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "choices[0].message",
            });
        }
    };
    Ok(ModerationChoice { message })
}

fn moderation_message(
    fields: &IndexMap<String, Json>,
) -> Result<ModerationMessage, ModerationError> {
    let content = match fields.get("content") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "choices[0].message.content",
            });
        }
    };
    let tool_calls = match fields.get("tool_calls") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(calls)) => calls
            .iter()
            .map(returned_tool_call)
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "choices[0].message.tool_calls",
            });
        }
    };
    Ok(ModerationMessage {
        content,
        tool_calls,
    })
}

fn returned_tool_call(value: &Json) -> Result<ReturnedToolCall, ModerationError> {
    let fields = object_fields(value, "choices[0].message.tool_calls")?;
    Ok(ReturnedToolCall {
        id: text_field(fields, "id")?.map(str::to_owned),
    })
}
