use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;
use crate::logging_privacy::PrivacyContext;
use crate::logging_privacy::request::{Flag, Metadata, PrivacyRequest};
use crate::logging_state::LoggingState;

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
pub fn redacted_standard_logging_payload(
    payload: &StandardLoggingPayload,
) -> StandardLoggingPayload {
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

// _redact_standard_logging_object's response branch: a string response becomes the marker itself.
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

// perform_redaction's result branch, which the logged response goes through before it is stored.
pub(crate) fn redact_response_text(response: Json) -> Json {
    match response {
        Json::Object(mut fields) if fields.contains_key("output") => {
            redact_output(&mut fields);
            Json::Object(fields)
        }
        Json::Object(mut fields) if fields.contains_key("choices") => {
            redact_choices(&mut fields);
            scrub_vertex_metadata(&mut fields);
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

fn redact_output(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(items)) = fields.get_mut("output") else {
        return;
    };
    for item in items {
        let Json::Object(item_fields) = item else {
            continue;
        };
        redact_present(item_fields, "text");
        if let Some(Json::List(content)) = item_fields.get_mut("content") {
            for part in content {
                if let Json::Object(part_fields) = part {
                    redact_present(part_fields, "text");
                    redact_present(part_fields, "refusal");
                }
            }
        }
        let kind = match item_fields.get("type") {
            Some(Json::Text(kind)) => kind.clone(),
            _ => String::new(),
        };
        if kind == "reasoning"
            && let Some(Json::List(summary)) = item_fields.get_mut("summary")
        {
            for entry in summary {
                if let Json::Object(entry_fields) = entry {
                    redact_present(entry_fields, "text");
                }
            }
        }
        if kind == "function_call" && item_fields.contains_key("arguments") {
            item_fields.insert("arguments".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
        }
        if kind == "custom_tool_call" && item_fields.contains_key("input") {
            item_fields.insert("input".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
        }
    }
}

fn redact_choices(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(choices)) = fields.get_mut("choices") else {
        return;
    };
    for choice in choices {
        let Json::Object(choice_fields) = choice else {
            continue;
        };
        let message_key = if matches!(choice_fields.get("message"), Some(Json::Object(_))) {
            Some("message")
        } else if matches!(choice_fields.get("delta"), Some(Json::Object(_))) {
            Some("delta")
        } else {
            None
        };
        if let Some(key) = message_key {
            if let Some(Json::Object(message)) = choice_fields.get_mut(key) {
                redact_message(message);
            }
        } else {
            redact_present(choice_fields, "text");
        }
    }
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

// should_redact_message_logging: the request flag, then the redaction headers, then the global setting.
#[must_use]
pub fn should_redact_message_logging(state: &LoggingState, settings: Settings) -> bool {
    let metadata = ["litellm_metadata", "metadata"].iter().find_map(|key| {
        match state.litellm_params.get(*key) {
            Some(Json::Object(fields)) => Some(fields),
            _ => None,
        }
    });
    let headers = match metadata.and_then(|fields| fields.get("headers")) {
        Some(Json::Object(headers)) => headers
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), flag(value)?)))
            .collect(),
        _ => IndexMap::new(),
    };
    let request = PrivacyRequest {
        direct: Metadata {
            turn_off_message_logging: state
                .callback_dynamic_params
                .get("turn_off_message_logging")
                .and_then(flag),
            headers,
            redact_user_api_key_info: None,
            callback_params: IndexMap::new(),
        },
        litellm_params: None,
        litellm_metadata: None,
        metadata: None,
    };
    match request.decide(settings, PrivacyContext::default(), true) {
        Ok(decision) => decision.content,
        Err(error) => {
            tracing::warn!(%error, "the message redaction flags could not be read");
            settings.turn_off_message_logging
        }
    }
}

fn flag(candidate: &Json) -> Option<Flag> {
    match candidate {
        Json::Bool(on) => Some(Flag::Boolean(*on)),
        Json::Text(text) => Some(Flag::Text(text.clone())),
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
