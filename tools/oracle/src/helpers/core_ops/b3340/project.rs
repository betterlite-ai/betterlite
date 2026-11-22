use std::collections::BTreeMap;

use betterlite_core::api::video_remix_list_transport::client::VideoRequest;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_state::LoggingState;
use serde_json::{Map, Value, json};

// The request the recording client saw: method, URL without query, sorted query pairs, the compared
// headers, the JSON body and the read timeout a caller named (the client's own timeout reads as none)
pub(crate) fn request(request: Option<&VideoRequest>, wanted: &[String]) -> Value {
    let Some(request) = request else {
        return Value::Null;
    };
    let mut query = request.query.clone();
    query.sort();
    let pairs: Vec<Value> = query
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect();
    let mut headers = Map::new();
    for (name, value) in &request.headers {
        let name = name.as_str().to_owned();
        if !wanted.contains(&name) {
            continue;
        }
        if let Ok(text) = value.to_str() {
            headers.insert(name, Value::String(text.to_owned()));
        }
    }
    let body = match &request.body {
        RequestBody::Json(bytes) => serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Value::Null
        }
    };
    json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "query": pairs,
        "headers": Value::Object(headers),
        "body": body,
        "timeout": request.timeout.map(|timeout| timeout.as_secs_f64()),
    })
}

// The header names a case compares: the provider's own two plus whatever the caller forwarded
pub(crate) fn wanted_headers(extra: &BTreeMap<String, String>) -> Vec<String> {
    let mut names = vec!["authorization".to_owned()];
    names.extend(extra.keys().map(|name| name.to_lowercase()));
    names
}

// The ten model_call_details keys LiteLLM writes around one video call, plus the call type attribute
pub(crate) fn logging(state: &LoggingState) -> Value {
    let details = serde_json::to_value(&state.model_call_details).unwrap_or_default();
    let at = |name: &str| details.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "call_type": state.call_type,
        "model": at("model"),
        "detail_call_type": at("call_type"),
        "user": at("user"),
        "custom_llm_provider": at("custom_llm_provider"),
        "litellm_call_id": at("litellm_call_id"),
        "optional_params": at("optional_params"),
        "video_id": at("video_id"),
        "prompt": at("prompt"),
        "after": at("after"),
        "limit": at("limit"),
        "order": at("order"),
    })
}

// What litellm.exception_type leaves on the wrapped error: status, type, param and code
pub(crate) fn error(class: &ErrorClass) -> Value {
    let metadata = class.metadata();
    json!({
        "status": metadata.status_code,
        "type": wire(&metadata.error_type),
        "param": wire(&metadata.param),
        "code": metadata.openai_code,
    })
}

fn wire(field: &WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text.clone()),
        WireField::Null | WireField::Missing(_) => None,
    }
}
