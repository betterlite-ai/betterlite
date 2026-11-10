use super::support::{Fields, as_object};
use crate::logging_privacy::request::Flag;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
const REDACTED_BY_LITELLM: &str = "redacted-by-litellm";
const DISABLE_HEADER: &str = "litellm-disable-message-redaction";
const VERTEX_METADATA_FIELDS: [&str; 5] = [
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_ratings",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
];
#[must_use]
pub fn should_redact_message_logging(
    litellm_params: Option<&Fields>,
    dynamic_params: Option<&Fields>,
    global: bool,
) -> bool {
    let metadata = request_metadata(litellm_params);
    if header_flag(metadata, DISABLE_HEADER) {
        return false;
    }
    if let Some(dynamic) = dynamic_params
        && let Some(flag) = dynamic.get("turn_off_message_logging").and_then(flag_value)
    {
        return flag;
    }
    header_flag(metadata, "litellm-enable-message-redaction")
        || header_flag(metadata, "x-litellm-enable-message-redaction")
        || global
}
fn request_metadata(litellm_params: Option<&Fields>) -> Option<&Fields> {
    let params = litellm_params?;
    let field = params
        .get("litellm_metadata")
        .map_or("metadata", |_| "litellm_metadata");
    params
        .get(field)
        .and_then(as_object)
        .or_else(|| params.get("metadata").and_then(as_object))
}
fn header_flag(metadata: Option<&Fields>, name: &str) -> bool {
    metadata
        .and_then(|bucket| bucket.get("headers"))
        .and_then(as_object)
        .and_then(|headers| headers.get(name).and_then(flag_value))
        .unwrap_or(false)
}
// B188's typed flag reader: a string must spell true or false, unlike Python truthiness
fn flag_value(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(flag) => Some(*flag),
        Json::Text(text) => Flag::Text(text.clone()).resolve().ok().flatten(),
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
pub fn redact_request_body(body: &mut Json) {
    let Json::Object(fields) = body else {
        return;
    };
    if let Some(Json::Object(params)) = fields.get_mut("litellm_params")
        && let Some(Json::Object(request)) = params.get_mut("proxy_server_request")
    {
        request.shift_remove("classifier_input");
        request.shift_remove("originating_request_masked");
    }
    let redacted = Json::Text(REDACTED_BY_LITELLM.to_owned());
    let message = IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), redacted),
    ]);
    fields.insert(
        "messages".to_owned(),
        Json::List(vec![Json::Object(message)]),
    );
    fields.insert("prompt".to_owned(), Json::Text(String::new()));
    fields.insert("input".to_owned(), Json::Text(String::new()));
    fields.shift_remove("served_output_texts");
    for key in ["metadata", "litellm_metadata"] {
        if let Some(Json::Object(metadata)) = fields.get_mut(key)
            && let Some(Json::Object(hidden)) = metadata.get_mut("hidden_params")
        {
            for field in VERTEX_METADATA_FIELDS {
                hidden.shift_remove(field);
            }
        }
    }
}
#[must_use]
pub fn redact_response(mut response: Json) -> Json {
    let Json::Object(fields) = &mut response else {
        return text_object();
    };
    if fields.contains_key("choices") {
        if let Some(Json::List(choices)) = fields.get_mut("choices") {
            redact_choices(choices);
        }
        for field in VERTEX_METADATA_FIELDS {
            if fields.contains_key(field) {
                fields.insert(field.to_owned(), Json::List(Vec::new()));
            }
        }
        if let Some(Json::Object(hidden)) = fields.get_mut("_hidden_params") {
            hidden.retain(|key, _| !VERTEX_METADATA_FIELDS.contains(&key.as_str()));
        }
        return response;
    }
    if fields.contains_key("output")
        && let Some(Json::List(items)) = fields.get_mut("output")
    {
        for item in items {
            if let Json::Object(output) = item {
                redact_output_item(output);
            }
        }
    }
    if fields.contains_key("output") {
        return response;
    }
    text_object()
}
fn text_object() -> Json {
    let text = Json::Text(REDACTED_BY_LITELLM.to_owned());
    let mut fields = IndexMap::new();
    fields.insert("text".to_owned(), text);
    Json::Object(fields)
}
fn redact_choices(choices: &mut [Json]) {
    for choice in choices {
        if let Json::Object(choice_fields) = choice {
            redact_choice(choice_fields);
        }
    }
}
fn redact_choice(fields: &mut Fields) {
    for role in ["message", "delta"] {
        if let Some(Json::Object(message)) = fields.get_mut(role) {
            redact_message(message);
            return;
        }
    }
    redact_present_text(fields, "text");
}
fn redact_message(message: &mut Fields) {
    for field in ["content", "reasoning_content"] {
        redact_present_text(message, field);
    }
    for field in ["thinking_blocks", "audio"] {
        if message.contains_key(field) {
            message.insert(field.to_owned(), Json::Null);
        }
    }
    if let Some(Json::List(tool_calls)) = message.get_mut("tool_calls") {
        for tool_call in tool_calls {
            if let Json::Object(call) = tool_call
                && let Some(Json::Object(function)) = call.get_mut("function")
            {
                function.insert(
                    "arguments".to_owned(),
                    Json::Text(REDACTED_BY_LITELLM.to_owned()),
                );
            }
        }
    }
    if let Some(Json::Object(function_call)) = message.get_mut("function_call") {
        set_text(function_call, "arguments");
    }
}
fn redact_output_item(fields: &mut Fields) {
    redact_present_text(fields, "text");
    if let Some(Json::List(content)) = fields.get_mut("content") {
        for content_item in content {
            if let Json::Object(entry) = content_item {
                redact_present_text(entry, "text");
                redact_present_text(entry, "refusal");
            }
        }
    }
    if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning")
        && let Some(Json::List(summary)) = fields.get_mut("summary")
    {
        for summary_item in summary {
            if let Json::Object(entry) = summary_item {
                redact_present_text(entry, "text");
            }
        }
    }
    if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "function_call") {
        set_text(fields, "arguments");
    }
    if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "custom_tool_call") {
        set_text(fields, "input");
    }
}
fn set_text(fields: &mut Fields, name: &str) {
    if fields.contains_key(name) {
        fields.insert(name.to_owned(), Json::Text(REDACTED_BY_LITELLM.to_owned()));
    }
}
fn redact_present_text(fields: &mut Fields, name: &str) {
    if let Some(value) = fields.get_mut(name)
        && !matches!(value, Json::Null)
    {
        *value = Json::Text(REDACTED_BY_LITELLM.to_owned());
    }
}
