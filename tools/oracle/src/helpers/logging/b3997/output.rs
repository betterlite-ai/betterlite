use betterlite_logging::gcs_client::gcs_storage_auth::RequestHeaders;
use betterlite_logging::gcs_mock::auth::{EnsureAccessToken, GetTokenAndUrl, VertexAuthProvider};
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

pub(super) fn headers_json(headers: &RequestHeaders) -> Value {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), json!(value.expose_secret())))
        .collect()
}

pub(super) fn secret_text(secret: Option<&SecretString>) -> Value {
    secret.map_or(Value::Null, |value| json!(value.expose_secret()))
}

pub(super) fn ensure_json(request: &EnsureAccessToken) -> Value {
    json!({
        "credentials": secret_text(request.credentials.as_ref()),
        "project_id": request.project_id,
        "custom_llm_provider": provider_name(request.custom_llm_provider),
    })
}

pub(super) fn token_json(request: &GetTokenAndUrl) -> Value {
    json!({
        "model": request.model,
        "auth_header": secret_text(request.auth_header.as_ref()),
        "vertex_credentials": secret_text(request.vertex_credentials.as_ref()),
        "vertex_project": request.vertex_project,
        "vertex_location": request.vertex_location,
        "gemini_api_key": secret_text(request.gemini_api_key.as_ref()),
        "stream": request.stream,
        "custom_llm_provider": provider_name(request.custom_llm_provider),
        "api_base": request.api_base,
    })
}

pub(super) const fn provider_name(provider: VertexAuthProvider) -> &'static str {
    match provider {
        VertexAuthProvider::VertexAi => "vertex_ai",
        VertexAuthProvider::VertexAiBeta => "vertex_ai_beta",
        VertexAuthProvider::Gemini => "gemini",
    }
}

pub(super) fn json_value(value: &Json) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

pub(super) fn parse_or_text(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_error| Value::String(text.to_owned()))
}

pub(super) fn body_json(body: Option<&[u8]>) -> Value {
    body.and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map_or(Value::Null, parse_or_text)
}

const PAYLOAD_KEYS: [&str; 6] = [
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
            for key in PAYLOAD_KEYS {
                kept.insert(
                    key.to_owned(),
                    fields.get(key).cloned().unwrap_or(Value::Null),
                );
            }
            Value::Object(kept)
        }
        other => other,
    }
}
