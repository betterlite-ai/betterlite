use betterlite_logging::gcs_client::gcs_storage_auth::RequestHeaders;
use betterlite_logging::gcs_mock::auth::{GetTokenAndUrl, VertexAuthProvider};
use secrecy::ExposeSecret;
use serde_json::{Map, Value, json};
use std::str;

pub(super) fn headers_json(headers: &RequestHeaders) -> Value {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), json!(value.expose_secret())))
        .collect()
}

pub(super) fn token_json(request: &GetTokenAndUrl) -> Value {
    json!({
        "model": request.model,
        "auth_header": json!(request.auth_header.as_ref().map(ExposeSecret::expose_secret)),
        "vertex_credentials": json!(request.vertex_credentials.as_ref().map(ExposeSecret::expose_secret)),
        "vertex_project": request.vertex_project,
        "vertex_location": request.vertex_location,
        "gemini_api_key": json!(request.gemini_api_key.as_ref().map(ExposeSecret::expose_secret)),
        "stream": request.stream,
        "custom_llm_provider": match request.custom_llm_provider {
            VertexAuthProvider::VertexAi => "vertex_ai",
            VertexAuthProvider::VertexAiBeta => "vertex_ai_beta",
            VertexAuthProvider::Gemini => "gemini",
        },
        "api_base": request.api_base,
    })
}

pub(super) fn body_json(body: Option<&[u8]>) -> Value {
    let Some(text) = body.and_then(|bytes| str::from_utf8(bytes).ok()) else {
        return Value::Null;
    };
    serde_json::from_str(text).unwrap_or_else(|_error| Value::String(text.to_owned()))
}

const KEPT_KEYS: [&str; 6] = [
    "id",
    "trace_id",
    "call_type",
    "status",
    "total_tokens",
    "model",
];

pub(super) fn project_payload(value: Value) -> Value {
    match value {
        Value::Object(fields) if fields.contains_key("id") && fields.contains_key("trace_id") => {
            let mut kept = Map::new();
            for key in KEPT_KEYS {
                let field_value = fields.get(key).cloned().unwrap_or(Value::Null);
                kept.insert(key.to_owned(), field_value);
            }
            Value::Object(kept)
        }
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_)) => other,
    }
}
