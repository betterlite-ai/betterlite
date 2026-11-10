use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::as_mapping;
use crate::logging_privacy::request::Flag;
use crate::spend_sanitize::value::{SpendMapping, SpendValue};

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
    litellm_params: Option<&SpendMapping>,
    dynamic_params: Option<&SpendValue>,
    global: bool,
) -> bool {
    let metadata = request_metadata(litellm_params);
    if header_flag(metadata, DISABLE_HEADER) {
        return false;
    }
    if let Some(dynamic) = dynamic_params.and_then(as_mapping)
        && let Some(flag) = dynamic
            .entries
            .get("turn_off_message_logging")
            .and_then(flag_value)
    {
        return flag;
    }
    if header_flag(metadata, "litellm-enable-message-redaction")
        || header_flag(metadata, "x-litellm-enable-message-redaction")
    {
        return true;
    }
    global
}

fn request_metadata(litellm_params: Option<&SpendMapping>) -> Option<&SpendMapping> {
    let params = litellm_params?;
    let field = if params.entries.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    params
        .entries
        .get(field)
        .and_then(as_mapping)
        .or_else(|| params.entries.get("metadata").and_then(as_mapping))
}

fn header_flag(metadata: Option<&SpendMapping>, name: &str) -> bool {
    metadata
        .and_then(|bucket| bucket.entries.get("headers"))
        .and_then(as_mapping)
        .and_then(|headers| headers.entries.get(name).and_then(flag_value))
        .unwrap_or(false)
}

// B188's typed flag reader: a string must spell true or false, unlike Python truthiness
fn flag_value(value: &SpendValue) -> Option<bool> {
    match value {
        SpendValue::Bool(flag) => Some(*flag),
        SpendValue::Text(text) => Flag::Text(text.clone()).resolve().ok().flatten(),
        SpendValue::Null
        | SpendValue::Number(_)
        | SpendValue::List(_)
        | SpendValue::Mapping(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => None,
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
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert(
        "content".to_owned(),
        Json::Text(REDACTED_BY_LITELLM.to_owned()),
    );
    fields.insert(
        "messages".to_owned(),
        Json::List(vec![Json::Object(message)]),
    );
    fields.insert("prompt".to_owned(), Json::Text(String::new()));
    fields.insert("input".to_owned(), Json::Text(String::new()));
    fields.shift_remove("served_output_texts");
    scrub_vertex_litellm_params(fields);
}

#[must_use]
pub fn redact_response(mut response: Json) -> Json {
    let Json::Object(fields) = &mut response else {
        return Json::Object(IndexMap::from([(
            "text".to_owned(),
            Json::Text(REDACTED_BY_LITELLM.to_owned()),
        )]));
    };
    if fields.contains_key("choices") {
        if let Some(Json::List(choices)) = fields.get_mut("choices") {
            redact_choices(choices);
        }
        scrub_vertex_logged_object(fields);
        return response;
    }
    if fields.contains_key("output") {
        if let Some(Json::List(items)) = fields.get_mut("output") {
            redact_output_items(items);
        }
        return response;
    }
    Json::Object(IndexMap::from([(
        "text".to_owned(),
        Json::Text(REDACTED_BY_LITELLM.to_owned()),
    )]))
}

fn redact_choices(choices: &mut [Json]) {
    for choice in choices {
        let Json::Object(fields) = choice else {
            continue;
        };
        if let Some(Json::Object(message)) = fields.get_mut("message") {
            redact_message(message);
            continue;
        }
        if let Some(Json::Object(delta)) = fields.get_mut("delta") {
            redact_message(delta);
            continue;
        }
        if let Some(text) = fields.get_mut("text")
            && !matches!(text, Json::Null)
        {
            *text = Json::Text(REDACTED_BY_LITELLM.to_owned());
        }
    }
}

fn redact_message(message: &mut IndexMap<String, Json>) {
    for field in ["content", "reasoning_content"] {
        if let Some(value) = message.get_mut(field)
            && !matches!(value, Json::Null)
        {
            *value = Json::Text(REDACTED_BY_LITELLM.to_owned());
        }
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
    if let Some(Json::Object(function_call)) = message.get_mut("function_call")
        && function_call.contains_key("arguments")
    {
        function_call.insert(
            "arguments".to_owned(),
            Json::Text(REDACTED_BY_LITELLM.to_owned()),
        );
    }
}

fn redact_output_items(items: &mut [Json]) {
    for item in items {
        let Json::Object(fields) = item else {
            continue;
        };
        redact_present_text(fields, "text");
        redact_output_content(fields);
        redact_output_summary(fields);
        if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "function_call")
            && fields.contains_key("arguments")
        {
            fields.insert(
                "arguments".to_owned(),
                Json::Text(REDACTED_BY_LITELLM.to_owned()),
            );
        }
        if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "custom_tool_call")
            && fields.contains_key("input")
        {
            fields.insert(
                "input".to_owned(),
                Json::Text(REDACTED_BY_LITELLM.to_owned()),
            );
        }
    }
}

fn redact_output_content(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(content)) = fields.get_mut("content") else {
        return;
    };
    for content_item in content {
        if let Json::Object(entry) = content_item {
            redact_present_text(entry, "text");
            redact_present_text(entry, "refusal");
        }
    }
}

fn redact_output_summary(fields: &mut IndexMap<String, Json>) {
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning") {
        return;
    }
    let Some(Json::List(summary)) = fields.get_mut("summary") else {
        return;
    };
    for summary_item in summary {
        if let Json::Object(entry) = summary_item {
            redact_present_text(entry, "text");
        }
    }
}

fn redact_present_text(fields: &mut IndexMap<String, Json>, name: &str) {
    if let Some(value) = fields.get_mut(name)
        && !matches!(value, Json::Null)
    {
        *value = Json::Text(REDACTED_BY_LITELLM.to_owned());
    }
}

fn scrub_vertex_logged_object(fields: &mut IndexMap<String, Json>) {
    for field in VERTEX_METADATA_FIELDS {
        if fields.contains_key(field) {
            fields.insert(field.to_owned(), Json::List(Vec::new()));
        }
    }
    if let Some(Json::Object(hidden)) = fields.get_mut("_hidden_params") {
        for field in VERTEX_METADATA_FIELDS {
            hidden.shift_remove(field);
        }
    }
}

fn scrub_vertex_litellm_params(fields: &mut IndexMap<String, Json>) {
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
