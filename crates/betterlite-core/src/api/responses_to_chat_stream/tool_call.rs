use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::fields::Fields;
use super::records::ChatToolCall;
use crate::api::response_chat_history::tool_fields::output_tool_fields;

const CONVERTED_INDEX: usize = 0;

pub enum CallShape<'a> {
    Added { event: &'a Fields, index: i64 },
    Done { index: i64 },
}

pub fn convert(
    item: &Fields,
    shape: CallShape<'_>,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let converted = output_tool_fields(item, CONVERTED_INDEX)?;
    let arguments = match shape {
        CallShape::Added { event, .. } if converted.function.arguments.is_empty() => {
            event_arguments(event)?
        }
        CallShape::Added { .. } | CallShape::Done { .. } => converted.function.arguments.clone(),
    };
    let name = match shape {
        CallShape::Added { .. } if converted.function.name.is_empty() => None,
        CallShape::Added { .. } | CallShape::Done { .. } => Some(converted.function.name.as_str()),
    };
    let index = match shape {
        CallShape::Added { index, .. } | CallShape::Done { index } => index,
    };
    let function = function_value(
        name,
        &arguments,
        converted.provider_specific_fields.as_ref(),
    )?;
    Ok(ChatToolCall {
        id: Some(converted.id),
        index: Some(index),
        kind: Some(converted.kind),
        function: Some(function),
        custom: None,
        provider_specific_fields: converted.provider_specific_fields,
        extra: Map::new(),
    })
}

pub fn delta_call(index: i64, arguments: &str) -> Result<ChatToolCall, ResponsesChatStreamError> {
    Ok(ChatToolCall {
        id: None,
        index: Some(index),
        kind: Some("function".to_owned()),
        function: Some(function_value(None, arguments, None)?),
        custom: None,
        provider_specific_fields: None,
        extra: Map::new(),
    })
}

pub fn function_value(
    name: Option<&str>,
    arguments: &str,
    provider: Option<&IndexMap<String, Json>>,
) -> Result<Value, ResponsesChatStreamError> {
    let mut fields = Map::new();
    fields.insert(
        "name".to_owned(),
        name.map_or(Value::Null, |text| Value::String(text.to_owned())),
    );
    fields.insert("arguments".to_owned(), Value::String(arguments.to_owned()));
    if let Some(present) = provider {
        let serialized =
            serde_json::to_value(present).map_err(|source| ResponsesChatStreamError::Json {
                message: source.to_string(),
            })?;
        fields.insert("provider_specific_fields".to_owned(), serialized);
    }
    Ok(Value::Object(fields))
}

fn event_arguments(event: &Fields) -> Result<String, ResponsesChatStreamError> {
    match event.get("arguments") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "arguments" }),
    }
}
