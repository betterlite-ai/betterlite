use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;

pub const REDACTED_TEXT: &str = "redacted-by-litellm";

pub(crate) const VERTEX_METADATA_FIELDS: [&str; 5] = [
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_ratings",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
];

// redact_messages.redacted_standard_logging_payload: a callback that batches failure payloads redacts them itself.
#[must_use]
pub fn redact_standard_payload(payload: &StandardLoggingPayload) -> StandardLoggingPayload {
    let mut redacted = payload.clone();
    redacted.classifier_input = None;
    redacted.originating_request_masked = None;
    if redacted
        .messages
        .as_ref()
        .is_some_and(|message| *message != Json::Null)
    {
        redacted.messages = Some(redacted_messages_json());
    }
    if let Some(response) = redacted.response.take() {
        redacted.response = Some(redact_logged_response(response));
    }
    redacted
}

// perform_redaction rewrites kwargs["messages"] to this marker before the payload reads them.
pub(crate) fn redacted_messages_json() -> Json {
    Json::List(vec![Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text(REDACTED_TEXT.to_owned())),
    ]))])
}

// _redact_standard_logging_object reads output first, then choices, and scrubs vertex metadata in both.
fn redact_logged_response(response: Json) -> Json {
    match response {
        Json::Object(mut fields) if fields.contains_key("output") => {
            redact_output(&mut fields);
            scrub_vertex_metadata(&mut fields);
            Json::Object(fields)
        }
        Json::Object(mut fields) if fields.contains_key("choices") => {
            redact_choices(&mut fields);
            scrub_vertex_metadata(&mut fields);
            Json::Object(fields)
        }
        Json::Text(_) => Json::Text(REDACTED_TEXT.to_owned()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            text_placeholder()
        }
    }
}

// perform_redaction's result branch reads choices before output, and only the choices branch scrubs.
pub(crate) fn redact_response_text(response: Json) -> Json {
    match response {
        Json::Object(mut fields) if fields.contains_key("choices") => {
            redact_choices(&mut fields);
            scrub_vertex_metadata(&mut fields);
            Json::Object(fields)
        }
        Json::Object(mut fields) if fields.contains_key("output") => {
            redact_output(&mut fields);
            Json::Object(fields)
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => text_placeholder(),
    }
}

fn text_placeholder() -> Json {
    Json::Object(IndexMap::from([(
        "text".to_owned(),
        Json::Text(REDACTED_TEXT.to_owned()),
    )]))
}

fn as_object_mut(value: &mut Json) -> Option<&mut IndexMap<String, Json>> {
    if let Json::Object(fields) = value {
        return Some(fields);
    }
    None
}

fn redact_output(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(items)) = fields.get_mut("output") else {
        return;
    };
    for item in items.iter_mut().filter_map(as_object_mut) {
        redact_output_item(item);
    }
}

fn redact_output_item(item: &mut IndexMap<String, Json>) {
    redact_present(item, "text");
    if let Some(Json::List(content)) = item.get_mut("content") {
        for part in content.iter_mut().filter_map(as_object_mut) {
            redact_present(part, "text");
            redact_present(part, "refusal");
        }
    }
    let kind = match item.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        _ => String::new(),
    };
    if kind == "reasoning"
        && let Some(Json::List(summary)) = item.get_mut("summary")
    {
        for entry in summary.iter_mut().filter_map(as_object_mut) {
            redact_present(entry, "text");
        }
    }
    if kind == "function_call" && item.contains_key("arguments") {
        item.insert("arguments".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
    }
    if kind == "custom_tool_call" && item.contains_key("input") {
        item.insert("input".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
    }
}

fn redact_choices(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(choices)) = fields.get_mut("choices") else {
        return;
    };
    for choice in choices {
        if let Json::Object(choice_fields) = choice {
            redact_choice(choice_fields);
        }
    }
}

fn redact_choice(choice: &mut IndexMap<String, Json>) {
    if let Some(Json::Object(message)) = choice.get_mut("message") {
        redact_message(message);
        return;
    }
    if let Some(Json::Object(delta)) = choice.get_mut("delta") {
        redact_message(delta);
        return;
    }
    redact_present(choice, "text");
}

fn redact_message(message: &mut IndexMap<String, Json>) {
    redact_present(message, "content");
    redact_present(message, "reasoning_content");
    for key in ["thinking_blocks", "audio"] {
        if let Some(value) = message.get_mut(key) {
            *value = Json::Null;
        }
    }
    if let Some(Json::List(calls)) = message.get_mut("tool_calls") {
        for call in calls {
            if let Json::Object(call_fields) = call
                && let Some(Json::Object(function)) = call_fields.get_mut("function")
            {
                function.insert("arguments".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
            }
        }
    }
    if let Some(Json::Object(call)) = message.get_mut("function_call")
        && call.contains_key("arguments")
    {
        call.insert("arguments".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
    }
}

fn redact_present(fields: &mut IndexMap<String, Json>, key: &str) {
    if let Some(value) = fields.get_mut(key)
        && *value != Json::Null
    {
        *value = Json::Text(REDACTED_TEXT.to_owned());
    }
}

fn scrub_vertex_metadata(fields: &mut IndexMap<String, Json>) {
    for name in VERTEX_METADATA_FIELDS {
        if let Some(value) = fields.get_mut(name) {
            *value = Json::List(Vec::new());
        }
        if let Some(Json::Object(hidden)) = fields.get_mut("_hidden_params") {
            hidden.shift_remove(name);
        }
    }
}
