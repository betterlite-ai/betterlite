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
        Some(Json::Object(message)) => Some(ModerationMessage {
            content: optional_content(message)?,
            tool_calls: returned_tool_calls(message)?,
        }),
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "choices[0].message",
            });
        }
    };
    Ok(ModerationChoice { message })
}

fn optional_content(fields: &IndexMap<String, Json>) -> Result<Option<String>, ModerationError> {
    let Some(value) = fields.get("content") else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ModerationError::FieldType {
                field: "choices[0].message.content",
            })
        }
    }
}

fn returned_tool_calls(
    fields: &IndexMap<String, Json>,
) -> Result<Vec<ReturnedToolCall>, ModerationError> {
    match fields.get("tool_calls") {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(calls)) => calls.iter().map(returned_tool_call).collect(),
        Some(_) => Err(ModerationError::FieldType {
            field: "choices[0].message.tool_calls",
        }),
    }
}

fn returned_tool_call(value: &Json) -> Result<ReturnedToolCall, ModerationError> {
    let fields = object_fields(value, "choices[0].message.tool_calls")?;
    Ok(ReturnedToolCall {
        id: text_field(fields, "id")?.map(str::to_owned),
    })
}
