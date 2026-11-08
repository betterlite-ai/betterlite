use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::payload::{CallbackKwargs, StandardLoggingPayload};

pub const MILLISECONDS_PER_SECOND: f64 = 1000.0;
pub const CACHE_HIT_SUFFIX: &str = "_cache_hit";
pub const TRACE_INGEST_ROUTE: &str = "/v1/traces";
const VERSION_HEX_DIGITS: usize = 2;
const TRACE_ID_HEX_DIGITS: usize = 32;
const SPAN_ID_HEX_DIGITS: usize = 16;
const FLAGS_HEX_DIGITS: usize = 2;

#[must_use]
pub fn strip_cache_hit_suffix(request_id: &str) -> String {
    let Some((prefix, suffix)) = request_id.rsplit_once(CACHE_HIT_SUFFIX) else {
        return request_id.to_owned();
    };
    let timestamp = suffix
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.');
    if timestamp {
        return prefix.to_owned();
    }
    request_id.to_owned()
}

#[must_use]
pub fn parse_traceparent(value: &str) -> (String, String) {
    let lowered = value.trim().to_ascii_lowercase();
    let mut segments = lowered.split('-');
    let (Some(version), Some(trace_id), Some(span_id), Some(flags), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return (String::new(), String::new());
    };
    let shaped = version.len() == VERSION_HEX_DIGITS
        && trace_id.len() == TRACE_ID_HEX_DIGITS
        && span_id.len() == SPAN_ID_HEX_DIGITS
        && flags.len() == FLAGS_HEX_DIGITS;
    let hexadecimal = [version, trace_id, span_id, flags]
        .iter()
        .all(|segment| segment.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !shaped || !hexadecimal {
        return (String::new(), String::new());
    }
    let zero_trace = trace_id.bytes().all(|byte| byte == b'0');
    let zero_span = span_id.bytes().all(|byte| byte == b'0');
    if zero_trace || zero_span {
        return (String::new(), String::new());
    }
    (trace_id.to_owned(), span_id.to_owned())
}

#[must_use]
pub fn to_ms(value: Option<&Json>) -> Option<i64> {
    let Some(Json::Number(seconds)) = value else {
        return None;
    };
    let milliseconds = seconds.as_f64()?.mul_add(MILLISECONDS_PER_SECOND, 0.0_f64);
    let rounded = Decimal::from_f64_retain(milliseconds)?;
    i64::try_from(rounded.trunc()).ok()
}

#[must_use]
pub fn int_value(value: Option<&Json>) -> i64 {
    match value {
        Some(Json::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

#[must_use]
pub fn json_value(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => String::new(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[must_use]
pub fn coerced_text(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(set) => (if *set { "True" } else { "False" }).to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn is_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(set) => *set,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|magnitude| magnitude != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn first_nonzero(values: &[i64]) -> i64 {
    for value in values {
        if *value != 0 {
            return *value;
        }
    }
    0
}

#[must_use]
pub fn cache_tokens(usage: &IndexMap<String, Json>) -> (i64, i64) {
    let empty = IndexMap::new();
    let details = match usage.get("prompt_tokens_details") {
        Some(Json::Object(details)) => details,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => &empty,
    };
    let cache_read = first_nonzero(&[
        int_value(details.get("cached_tokens")),
        int_value(usage.get("cache_read_input_tokens")),
    ]);
    let cache_write = first_nonzero(&[
        int_value(details.get("cache_write_tokens")),
        int_value(details.get("cache_creation_tokens")),
        int_value(usage.get("cache_creation_input_tokens")),
    ]);
    (cache_read, cache_write)
}

#[must_use]
pub fn request_tags(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::List(tags)) => tags.iter().map(coerced_text).collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Vec::new(),
    }
}

#[must_use]
pub fn session_id(payload: &StandardLoggingPayload, kwargs: &CallbackKwargs) -> String {
    if let Some(session) = payload
        .session_id
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        return session.to_owned();
    }
    let from_metadata = kwargs
        .litellm_params
        .as_ref()
        .and_then(|params| params.metadata.as_ref())
        .and_then(|metadata| metadata.get("session_id"))
        .filter(|value| is_present(value))
        .map(coerced_text);
    if let Some(session) = from_metadata {
        return session;
    }
    payload.trace_id.clone().unwrap_or_default()
}

#[must_use]
pub fn find_traceparent(
    metadata: Option<&IndexMap<String, Json>>,
    kwargs: &CallbackKwargs,
) -> (String, String) {
    let custom_headers = metadata.and_then(|fields| fields.get("requester_custom_headers"));
    if let Some(Json::Object(headers)) = custom_headers {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("traceparent") {
                return match value {
                    Json::Text(text) => parse_traceparent(text),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => (String::new(), String::new()),
                };
            }
        }
    }
    let proxy_headers = kwargs
        .litellm_params
        .as_ref()
        .and_then(|params| params.proxy_server_request.as_ref())
        .and_then(|request| request.headers.as_ref());
    if let Some(headers) = proxy_headers {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("traceparent") {
                return parse_traceparent(value);
            }
        }
    }
    (String::new(), String::new())
}

mod tests;
