use betterlite_models::decisions::Json;

use super::error::NativeCompactionError;
use super::protocol::{CompactionProtocol, Mapping};

fn objects<'response>(
    value: Option<&'response Json>,
    field: &'static str,
) -> Result<Vec<&'response Mapping>, NativeCompactionError> {
    match value {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Object(object) => Ok(object),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    Err(NativeCompactionError::NotAnObjectList { field })
                }
            })
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(NativeCompactionError::NotAnObjectList { field })
        }
    }
}

const fn object<'response>(
    value: Option<&'response Json>,
    field: &'static str,
) -> Result<Option<&'response Mapping>, NativeCompactionError> {
    if let Some(Json::Object(found)) = value {
        return Ok(Some(found));
    }
    if matches!(value, None | Some(Json::Null)) {
        Ok(None)
    } else {
        Err(NativeCompactionError::NotAnObject { field })
    }
}

// A messages reply carries its blocks only when it stopped for compaction; a chat reply needs exactly one choice.
pub fn native_blocks(
    protocol: CompactionProtocol,
    response: &Mapping,
) -> Result<Vec<&Mapping>, NativeCompactionError> {
    if protocol == CompactionProtocol::Messages {
        let stopped = matches!(response.get("stop_reason"), Some(Json::Text(reason)) if reason == "compaction");
        return if stopped {
            objects(response.get("content"), "content")
        } else {
            Ok(Vec::new())
        };
    }
    let choices = objects(response.get("choices"), "choices")?;
    let [choice] = choices.as_slice() else {
        return Ok(Vec::new());
    };
    let Some(message) = object(choice.get("message"), "message")? else {
        return Ok(Vec::new());
    };
    let Some(fields) = object(
        message.get("provider_specific_fields"),
        "provider_specific_fields",
    )?
    else {
        return Ok(Vec::new());
    };
    objects(fields.get("compaction_blocks"), "compaction_blocks")
}
