use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::stream_state::state::StreamState;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};

pub(crate) type HelperValue = Result<Value, Box<HelperError>>;

pub(crate) fn header_map(pairs: &IndexMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::try_from(name.as_str()).or_value_error()?,
            HeaderValue::try_from(value.as_str()).or_value_error()?,
        );
    }
    Ok(headers)
}

pub(crate) fn selected(headers: &HeaderMap, names: &[String]) -> Value {
    let mut fields = serde_json::Map::new();
    for name in names {
        let value = headers
            .get(name.as_str())
            .and_then(|value| value.to_str().ok())
            .map_or(Value::Null, |text| Value::String(text.to_owned()));
        fields.insert(name.clone(), value);
    }
    Value::Object(fields)
}

pub(crate) fn post_value(post: &StreamPost, names: &[String]) -> Value {
    json!({
        "url": post.url.as_str(),
        "body": String::from_utf8(post.body.clone())
            .unwrap_or_else(|error| format!("{} bytes that are not text", error.as_bytes().len())),
        "request_headers": selected(&post.headers, names),
    })
}

pub(crate) fn options_value(state: &StreamState) -> Value {
    let mut reverse = serde_json::Map::new();
    for (sanitized, original) in &state.tool_name_reverse_map {
        reverse.insert(sanitized.clone(), Value::String(original.clone()));
    }
    json!({
        "json_mode": state.json_mode,
        "speed": state.speed,
        "tool_name_reverse_map": Value::Object(reverse),
    })
}
