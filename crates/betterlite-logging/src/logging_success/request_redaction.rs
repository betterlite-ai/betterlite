use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::payload_state::{set_standard_logging_object, standard_logging_object};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_privacy::PrivacyContext;
use crate::logging_privacy::request::{Flag, Metadata, PrivacyRequest};
use crate::logging_state::LoggingState;

const REDACTED: &str = "redacted-by-litellm";
const VERTEX_METADATA_FIELDS: [&str; 5] = [
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_ratings",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
];

// should_redact_message_logging: the request flag, then the redaction headers, then the global setting
pub(super) fn should_redact_message_logging(state: &LoggingState, settings: Settings) -> bool {
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

pub(super) fn redact_stored_payload(state: &mut LoggingState) {
    let mut payload = match standard_logging_object(state) {
        Ok(Some(payload)) => payload,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "the standard logging payload could not be read");
            return;
        }
    };
    redact_payload(&mut payload);
    if let Err(error) = set_standard_logging_object(state, Some(&payload)) {
        tracing::warn!(%error, "the redacted standard logging payload could not be stored");
    }
}

pub(super) fn redact_payload(payload: &mut StandardLoggingPayload) {
    payload.classifier_input = None;
    payload.originating_request_masked = None;
    if payload
        .messages
        .as_ref()
        .is_some_and(|messages| *messages != Json::Null)
    {
        let message = IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text(REDACTED.to_owned())),
        ]);
        payload.messages = Some(Json::List(vec![Json::Object(message)]));
    }
    if let Some(response) = payload.response.as_mut() {
        redact_response(response);
    }
}

fn redact_response(response: &mut Json) {
    match response {
        Json::Null => {}
        Json::Object(fields) if fields.contains_key("output") => {
            for item in objects(fields, "output") {
                redact_output_item(item);
            }
            scrub_vertex_metadata(fields);
        }
        Json::Object(fields) if fields.contains_key("choices") => {
            for choice in objects(fields, "choices") {
                redact_choice(choice);
            }
            scrub_vertex_metadata(fields);
        }
        Json::Text(text) => REDACTED.clone_into(text),
        Json::Object(_) | Json::List(_) | Json::Bool(_) | Json::Number(_) => {
            *response = Json::Object(IndexMap::from([(
                "text".to_owned(),
                Json::Text(REDACTED.to_owned()),
            )]));
        }
    }
}

fn objects<'fields>(
    fields: &'fields mut IndexMap<String, Json>,
    key: &str,
) -> impl Iterator<Item = &'fields mut IndexMap<String, Json>> {
    let items = match fields.get_mut(key) {
        Some(Json::List(items)) => items.as_mut_slice(),
        _ => &mut [],
    };
    items.iter_mut().filter_map(|item| {
        if let Json::Object(object) = item {
            Some(object)
        } else {
            None
        }
    })
}

fn redact_present(fields: &mut IndexMap<String, Json>, key: &str) {
    if let Some(value) = fields.get_mut(key)
        && *value != Json::Null
    {
        *value = Json::Text(REDACTED.to_owned());
    }
}

fn redact_choice(choice: &mut IndexMap<String, Json>) {
    for key in ["message", "delta"] {
        if let Some(Json::Object(message)) = choice.get_mut(key) {
            redact_message(message);
            return;
        }
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
    for call in objects(message, "tool_calls") {
        if let Some(Json::Object(function)) = call.get_mut("function") {
            function.insert("arguments".to_owned(), Json::Text(REDACTED.to_owned()));
        }
    }
    if let Some(Json::Object(call)) = message.get_mut("function_call")
        && call.contains_key("arguments")
    {
        call.insert("arguments".to_owned(), Json::Text(REDACTED.to_owned()));
    }
}

fn redact_output_item(item: &mut IndexMap<String, Json>) {
    redact_present(item, "text");
    for part in objects(item, "content") {
        redact_present(part, "text");
        redact_present(part, "refusal");
    }
    let kind = match item.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        _ => String::new(),
    };
    match kind.as_str() {
        "reasoning" => {
            for summary in objects(item, "summary") {
                redact_present(summary, "text");
            }
        }
        "function_call" if item.contains_key("arguments") => {
            item.insert("arguments".to_owned(), Json::Text(REDACTED.to_owned()));
        }
        "custom_tool_call" if item.contains_key("input") => {
            item.insert("input".to_owned(), Json::Text(REDACTED.to_owned()));
        }
        _ => {}
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
