use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::python_json;

// Pydantic's model_dump_json() default separators have no surrounding whitespace, unlike python_json::dumps's json.dumps-style defaults; scalars still go through python_json::dumps for escaping.
fn compact_json(value: &Json) -> String {
    match value {
        Json::List(items) => {
            let parts: Vec<String> = items.iter().map(compact_json).collect();
            format!("[{}]", parts.join(","))
        }
        Json::Object(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}:{}",
                        python_json::dumps(&Json::Text(key.clone())),
                        compact_json(item)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            python_json::dumps(scalar)
        }
    }
}

// litellm.types.utils.Function.__init__ (types/utils.py:1186-1203): object arguments become their json.dumps string, a missing or null arguments reads as "", and the declared field order (arguments, then name) holds regardless of the caller's own key order.
fn ordered_function_fields(function: &Json) -> Json {
    let Json::Object(fields) = function else {
        return function.clone();
    };
    let mut ordered = IndexMap::new();
    let arguments = match fields.get("arguments") {
        Some(dict @ Json::Object(_)) => Json::Text(python_json::dumps(dict)),
        Some(Json::Null) | None => Json::Text(String::new()),
        Some(other) => other.clone(),
    };
    ordered.insert("arguments".to_owned(), arguments);
    ordered.insert(
        "name".to_owned(),
        fields.get("name").cloned().unwrap_or(Json::Null),
    );
    Json::Object(ordered)
}

// litellm.types.utils.is_custom_tool_call_dict: a tool call is custom when its type is "custom" or it carries a non-null "custom" payload.
fn is_custom_tool_call(fields: &IndexMap<String, Json>) -> bool {
    matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "custom")
        || fields
            .get("custom")
            .is_some_and(|value| !matches!(value, Json::Null))
}

// litellm.types.utils.ChatCompletionMessageCustomToolCall.custom's declared field order (name, input).
fn custom_name_input_fields(custom_value: Option<&Json>) -> Json {
    match custom_value {
        Some(Json::Object(custom_fields)) => {
            let mut ordered_custom = IndexMap::new();
            ordered_custom.insert(
                "name".to_owned(),
                custom_fields.get("name").cloned().unwrap_or(Json::Null),
            );
            ordered_custom.insert(
                "input".to_owned(),
                custom_fields.get("input").cloned().unwrap_or(Json::Null),
            );
            Json::Object(ordered_custom)
        }
        _ => Json::Object(IndexMap::new()),
    }
}

// litellm.types.utils.chat_completion_tool_call_from_dict dispatches between the custom-shaped dump (ChatCompletionMessageCustomToolCall's declared field order: id, type, custom with name and input) and the function-shaped dump. ChatCompletionMessageToolCall.__init__ passes **params (the caller's extra keys, like "index") to super().__init__() before assigning self.function/id/type, so on a model with no declared fields those extras land in __pydantic_extra__ ahead of function, id and type.
fn ordered_tool_call_fields(tool_call: &Json) -> Json {
    let Json::Object(fields) = tool_call else {
        return tool_call.clone();
    };
    let mut ordered = IndexMap::new();
    if is_custom_tool_call(fields) {
        if let Some(id) = fields.get("id") {
            ordered.insert("id".to_owned(), id.clone());
        }
        ordered.insert("type".to_owned(), Json::Text("custom".to_owned()));
        ordered.insert(
            "custom".to_owned(),
            custom_name_input_fields(fields.get("custom")),
        );
        return Json::Object(ordered);
    }
    for (key, value) in fields {
        if key != "function" && key != "id" && key != "type" {
            ordered.insert(key.clone(), value.clone());
        }
    }
    let function = fields
        .get("function")
        .map_or_else(|| Json::Object(IndexMap::new()), ordered_function_fields);
    ordered.insert("function".to_owned(), function);
    if let Some(id) = fields.get("id") {
        ordered.insert("id".to_owned(), id.clone());
    }
    ordered.insert(
        "type".to_owned(),
        fields
            .get("type")
            .cloned()
            .unwrap_or_else(|| Json::Text("function".to_owned())),
    );
    Json::Object(ordered)
}

// litellm's get_content_from_model_response rebuilds the raw dict into a ModelResponse pydantic object before reading it back, so tool_calls/function_call come back in that class's field order, not the caller's.
fn push_message_content(content: &mut String, message: &IndexMap<String, Json>) {
    if let Some(Json::Text(text)) = message.get("content") {
        content.push_str(text);
    }
    if let Some(function_call) = message.get("function_call")
        && super::super::support::has_value(function_call)
    {
        content.push_str(&compact_json(&ordered_function_fields(function_call)));
    }
    if let Some(Json::List(tool_calls)) = message.get("tool_calls") {
        for tool_call in tool_calls {
            content.push_str(&compact_json(&ordered_tool_call_fields(tool_call)));
        }
    }
}

// litellm's ModelResponse(**response) turns every dict choice into Choices(**choice) with an empty Message() when no "message" key is given, so a StreamingChoices delta never contributes content through this raw-dict path.
fn push_choice_content(content: &mut String, choice: &Json) {
    let Json::Object(fields) = choice else {
        return;
    };
    if let Some(Json::Object(message)) = fields.get("message") {
        push_message_content(content, message);
    }
}

pub(super) fn content_from_dict_response(value: &IndexMap<String, Json>) -> String {
    let Some(Json::List(choices)) = value.get("choices") else {
        return String::new();
    };
    let mut content = String::new();
    for choice in choices {
        push_choice_content(&mut content, choice);
    }
    content
}

// The Message fields litellm deletes from the model (so they're absent from model_dump) whenever the caller's own argument was None, kept otherwise even if falsy (an empty list stays); provider_specific_fields and annotations are handled on their own right after.
const MESSAGE_KEPT_WHEN_NOT_NULL: [&str; 5] = [
    "audio",
    "images",
    "reasoning_content",
    "thinking_blocks",
    "reasoning_items",
];

// Every Message field litellm's __init__ reads by name; any other key the caller passed goes to **params, which SafeAttributeModel stores as pydantic extra fields appended after every declared field, in the caller's own order.
const MESSAGE_DECLARED_FIELDS: [&str; 11] = [
    "content",
    "role",
    "tool_calls",
    "function_call",
    "audio",
    "images",
    "reasoning_content",
    "thinking_blocks",
    "reasoning_items",
    "provider_specific_fields",
    "annotations",
];

// Everything model_dump places after function_call: audio/images/reasoning_content/thinking_blocks/reasoning_items when not null, provider_specific_fields (litellm.types.utils.add_provider_specific_fields keeps its None default unless the caller's dict is non-empty), annotations when not null, then the caller's own extra keys in their own order.
fn append_message_tail_fields(
    ordered: &mut IndexMap<String, Json>,
    fields: &IndexMap<String, Json>,
) {
    for key in MESSAGE_KEPT_WHEN_NOT_NULL {
        if let Some(value) = fields.get(key).filter(|value| !matches!(value, Json::Null)) {
            ordered.insert(key.to_owned(), value.clone());
        }
    }
    ordered.insert(
        "provider_specific_fields".to_owned(),
        fields
            .get("provider_specific_fields")
            .filter(|value| super::super::support::has_value(value))
            .cloned()
            .unwrap_or(Json::Null),
    );
    if let Some(annotations) = fields
        .get("annotations")
        .filter(|value| !matches!(value, Json::Null))
    {
        ordered.insert("annotations".to_owned(), annotations.clone());
    }
    for (key, value) in fields {
        if !MESSAGE_DECLARED_FIELDS.contains(&key.as_str()) {
            ordered.insert(key.clone(), value.clone());
        }
    }
}

// litellm.types.utils.Message.__init__ (types/utils.py:1399-1497): content, role, tool_calls and function_call are always explicitly set (nulls included).
pub(super) fn message_model_dump(message: &Json) -> Json {
    let empty = IndexMap::new();
    let fields = match message {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
    };
    let mut ordered = IndexMap::new();
    ordered.insert(
        "content".to_owned(),
        fields.get("content").cloned().unwrap_or(Json::Null),
    );
    ordered.insert(
        "role".to_owned(),
        fields
            .get("role")
            .filter(|value| super::super::support::has_value(value))
            .cloned()
            .unwrap_or_else(|| Json::Text("assistant".to_owned())),
    );
    ordered.insert(
        "tool_calls".to_owned(),
        match fields.get("tool_calls") {
            Some(Json::List(items)) if !items.is_empty() => {
                Json::List(items.iter().map(ordered_tool_call_fields).collect())
            }
            _ => Json::Null,
        },
    );
    ordered.insert(
        "function_call".to_owned(),
        match fields.get("function_call") {
            Some(value) if super::super::support::has_value(value) => {
                ordered_function_fields(value)
            }
            _ => Json::Null,
        },
    );
    append_message_tail_fields(&mut ordered, fields);
    Json::Object(ordered)
}

// litellm.types.utils.ImageObject.json() calls model_dump() directly (not the exclude_unset default), so its fields are always present, nulls included, in the order ImageObject's base class (openai.types.image.Image) declares b64_json and revised_prompt before url, then ImageObject itself adds provider_specific_fields.
pub(super) fn image_object_model_dump(image: &Json) -> Json {
    let empty = IndexMap::new();
    let fields = match image {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
    };
    let mut ordered = IndexMap::new();
    for key in [
        "b64_json",
        "revised_prompt",
        "url",
        "provider_specific_fields",
    ] {
        ordered.insert(
            key.to_owned(),
            fields.get(key).cloned().unwrap_or(Json::Null),
        );
    }
    Json::Object(ordered)
}
