use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::records::ChatToolCall;
use crate::api::response_chat_history::tool_fields::output_tool_fields;

// the conversion's own index is overwritten by the chat index, so its value carries no meaning here
const CONVERTED_INDEX: usize = 0;

pub fn added_tool_call(
    event: &Map<String, Value>,
    item: &Map<String, Value>,
    tool_call_index: i64,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let converted = output_tool_fields(item, CONVERTED_INDEX)?;
    let arguments = if converted.function.arguments.is_empty() {
        event_arguments(event)?
    } else {
        converted.function.arguments.clone()
    };
    let name = converted.function.name;
    let function = function_value(
        if name.is_empty() { None } else { Some(&name) },
        &arguments,
        converted.provider_specific_fields.as_ref(),
    )?;
    Ok(ChatToolCall {
        id: Some(converted.id.clone()),
        index: Some(tool_call_index),
        kind: Some(converted.kind.clone()),
        function: Some(function),
        custom: None,
        provider_specific_fields: converted.provider_specific_fields,
        extra: Map::new(),
    })
}

pub fn done_tool_call(
    item: &Map<String, Value>,
    output_index: i64,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let converted = output_tool_fields(item, CONVERTED_INDEX)?;
    let function = function_value(
        Some(&converted.function.name),
        &converted.function.arguments,
        converted.provider_specific_fields.as_ref(),
    )?;
    Ok(ChatToolCall {
        id: Some(converted.id),
        index: Some(output_index),
        kind: Some(converted.kind),
        function: Some(function),
        custom: None,
        provider_specific_fields: converted.provider_specific_fields,
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

// the event's own arguments stand in when the converted call carries none
fn event_arguments(event: &Map<String, Value>) -> Result<String, ResponsesChatStreamError> {
    match event.get("arguments") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "arguments" }),
    }
}
