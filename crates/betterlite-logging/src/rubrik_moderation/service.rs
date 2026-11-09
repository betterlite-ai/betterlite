use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik::block_decision::{
    ModerationChoice, ModerationMessage, ModerationServiceResponse, ReturnedToolCall,
};

use super::error::ModerationError;

// Reads the webhook's OpenAI-shaped answer the way the block checks do: only choices[0] is ever
// consulted, so a later choice with an unexpected shape cannot fail a request over nothing.
pub fn service_response(
    fields: &IndexMap<String, Json>,
) -> Result<ModerationServiceResponse, ModerationError> {
    let choices = match fields.get("choices") {
        None | Some(Json::Null) => &[][..],
        Some(Json::List(choices)) => choices.as_slice(),
        Some(_) => {
            return Err(ModerationError::FieldType { field: "choices" });
        }
    };
    let first = choices.first().map(moderation_choice).transpose()?;
    Ok(ModerationServiceResponse {
        choices: first.into_iter().collect(),
    })
}

fn moderation_choice(value: &Json) -> Result<ModerationChoice, ModerationError> {
    let Json::Object(fields) = value else {
        return Err(ModerationError::FieldType {
            field: "choices[0]",
        });
    };
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
    let Json::Object(fields) = value else {
        return Err(ModerationError::FieldType {
            field: "choices[0].message.tool_calls",
        });
    };
    let id = match fields.get("id") {
        None | Some(Json::Null) => None,
        Some(Json::Text(id)) => Some(id.clone()),
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "choices[0].message.tool_calls.id",
            });
        }
    };
    Ok(ReturnedToolCall { id })
}
