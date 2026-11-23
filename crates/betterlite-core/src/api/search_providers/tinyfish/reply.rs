use super::error::TinyfishSearchError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use serde_json::from_value;
use tracing::warn;
pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;
const DOCS_URL: &str = "https://docs.tinyfish.ai/search-api";
const PREVIEW_CHARS: usize = 200;
const RESULT_FIELDS: [&str; 3] = ["title", "url", "snippet"];
const WARNINGS_KEY: &str = "parameter_warnings";
const HEADERS_KEY: &str = "headers";
const ADDITIONAL_HEADERS_KEY: &str = "additional_headers";
pub fn decode_tinyfish_search(
    text: &str,
    status_code: u16,
    headers: &Headers,
    caller_limit: usize,
) -> Result<SearchResponse, TinyfishSearchError> {
    let fail = |message: &str| {
        TinyfishSearchError::Provider(Box::new(wrapped_error(message, status_code, headers)))
    };
    if !(200..300).contains(&status_code) {
        return Err(fail(text));
    }
    if text.len() > REPLY_MAX_BYTES {
        return Err(TinyfishSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let Ok(mut raw) = serde_json::from_str::<JsonValue>(text) else {
        let preview: String = text.chars().take(PREVIEW_CHARS).collect();
        return Err(fail(&format!("Expected JSON response, got: {preview}")));
    };
    default_result_fields(&mut raw);
    let mut reply: SearchResponse = match from_value(raw) {
        Ok(reply) => reply,
        Err(source) => {
            return Err(fail(&format!(
                "Response shape does not match LiteLLM's SearchResponse schema: {source}"
            )));
        }
    };
    emit_parameter_warnings(&reply);
    reply.results.truncate(caller_limit);
    reply.hidden_params = hidden_params(headers);
    Ok(reply)
}
pub(super) fn wrapped_error(text: &str, status_code: u16, headers: &Headers) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: format!(
            "TinyFish Search: {}. See {DOCS_URL} for details.",
            inner_message(text)
        ),
        headers: Some(headers.clone().into_iter().collect()),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
fn inner_message(text: &str) -> String {
    let Ok(JsonValue::Object(fields)) = serde_json::from_str::<JsonValue>(text) else {
        return text.to_owned();
    };
    let Some(JsonValue::Object(error)) = fields.get("error") else {
        return text.to_owned();
    };
    error
        .get("message")
        .and_then(JsonValue::as_str)
        .filter(|message| !message.is_empty())
        .map_or_else(|| text.to_owned(), str::to_owned)
}
fn default_result_fields(value: &mut JsonValue) {
    let Some(JsonValue::Array(results)) = value.get_mut("results") else {
        return;
    };
    for item in results {
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        for field in RESULT_FIELDS {
            if object.get(field).is_none_or(|current| !current.is_string()) {
                object.insert(field.to_owned(), JsonValue::String(String::new()));
            }
        }
    }
}
fn emit_parameter_warnings(reply: &SearchResponse) {
    let Some(JsonValue::Array(entries)) = reply.extra.get(WARNINGS_KEY) else {
        return;
    };
    for entry in entries {
        let Some(fields) = entry.as_object() else {
            continue;
        };
        let field = |name: &str| {
            fields
                .get(name)
                .and_then(JsonValue::as_str)
                .filter(|text| !text.is_empty())
        };
        let (Some(warning_type), Some(parameter), Some(message)) =
            (field("type"), field("parameter"), field("message"))
        else {
            continue;
        };
        warn!("TinyFish Search parameter_warning ({warning_type}) `{parameter}`: {message}");
    }
}
fn hidden_params(headers: &Headers) -> Object {
    let mut upstream: IndexMap<String, String> = IndexMap::with_capacity(headers.len());
    for (name, value) in headers {
        upstream.insert(name.clone(), value.clone());
    }
    let mut raw = Object::new();
    for (name, value) in &upstream {
        raw.insert(name.clone(), JsonValue::String(value.clone()));
    }
    let additional =
        serde_json::to_value(process_response_headers(HeaderSource::Upstream(&upstream)))
            .unwrap_or_default();
    let mut hidden = Object::new();
    hidden.insert(String::from(HEADERS_KEY), JsonValue::Object(raw));
    hidden.insert(String::from(ADDITIONAL_HEADERS_KEY), additional);
    hidden
}
