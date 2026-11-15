use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::fields::Fields;
use super::records::ChatToolCall;
use crate::api::response_chat_history::tool_fields::output_tool_fields;

const CONVERTED_INDEX: usize = 0;

pub(super) enum CallShape<'a> {
    Added { event: &'a Fields, index: i64 },
    Done { index: i64 },
}

pub(super) fn convert(
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
    let function = function_value(name, &arguments);
    Ok(ChatToolCall {
        id: Some(Some(converted.id)),
        index: Some(index),
        kind: Some(Some(converted.kind)),
        function: Some(function),
        custom: None,
        provider_specific_fields: converted.provider_specific_fields,
        extra: Map::new(),
    })
}

pub(super) fn delta_call(
    index: i64,
    arguments: &str,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    Ok(ChatToolCall {
        id: Some(None),
        index: Some(index),
        kind: Some(Some("function".to_owned())),
        function: Some(function_value(None, arguments)),
        custom: None,
        provider_specific_fields: None,
        extra: Map::new(),
    })
}

// Function.__init__ sets both name and arguments, so an unset name still dumps as null
pub(super) fn function_value(name: Option<&str>, arguments: &str) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "name".to_owned(),
        name.map_or(Value::Null, |text| Value::String(text.to_owned())),
    );
    fields.insert("arguments".to_owned(), Value::String(arguments.to_owned()));
    Value::Object(fields)
}

fn event_arguments(event: &Fields) -> Result<String, ResponsesChatStreamError> {
    match event.get("arguments") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "arguments" }),
    }
}
