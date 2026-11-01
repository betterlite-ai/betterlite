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

// litellm.types.utils.Function/FunctionCall's declared pydantic field order (arguments, then name), which model_dump_json() serializes by regardless of the caller's own key order.
fn ordered_function_fields(function: &Json) -> Json {
    let Json::Object(fields) = function else {
        return function.clone();
    };
    let mut ordered = IndexMap::new();
    ordered.insert(
        "arguments".to_owned(),
        fields
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    ordered.insert(
        "name".to_owned(),
        fields.get("name").cloned().unwrap_or(Json::Null),
    );
    Json::Object(ordered)
}

// litellm.types.utils.ChatCompletionMessageToolCall.__init__'s attribute-assignment order (function, then id, then type), which model_dump_json() serializes these extra="allow" fields by.
fn ordered_tool_call_fields(tool_call: &Json) -> Json {
    let Json::Object(fields) = tool_call else {
        return tool_call.clone();
    };
    let mut ordered = IndexMap::new();
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

fn push_delta_content(content: &mut String, fields: &IndexMap<String, Json>) {
    if let Some(Json::Object(delta)) = fields.get("delta")
        && let Some(Json::Text(text)) = delta.get("content")
    {
        content.push_str(text);
    }
}

fn push_choice_content(content: &mut String, choice: &Json) {
    let Json::Object(fields) = choice else {
        return;
    };
    match fields.get("message") {
        Some(Json::Object(message)) => push_message_content(content, message),
        _ => push_delta_content(content, fields),
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
