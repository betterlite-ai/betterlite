use betterlite_config::sections::embedding_host_settings::Settings;
use betterlite_prompts::media::base64_string::{get_base64_str, is_base64_encoded};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::embedding::{EmbeddingError, EmbeddingResponse};
use crate::openai_base_config::{Headers, Object};

pub mod error;

pub const DEFAULT_API_BASE: &str = "https://api.jina.ai/v1";
pub const DEFAULT_URL: &str = "https://api.jina.ai/v1/embeddings";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];

#[derive(Debug, Clone)]
pub struct JinaCredentials {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

// LiteLLM passes Python's builtin input function to post_call where the request text was meant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinInput;

#[derive(Debug)]
pub struct PostCall<'call> {
    pub input: BuiltinInput,
    pub api_key: Option<&'call SecretString>,
    pub complete_input_dict: &'call Object,
    pub original_response: &'call Value,
}

// an empty explicit value falls through to the environment, as LiteLLM's `or` chain does
#[must_use]
pub fn resolve_credentials(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> JinaCredentials {
    let resolved_base = api_base
        .filter(|base| !base.is_empty())
        .or(settings.jina_ai_api_base.as_deref())
        .unwrap_or(DEFAULT_API_BASE);
    let resolved_key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(settings.jina_ai_api_key.as_ref());
    JinaCredentials {
        api_base: resolved_base.to_owned(),
        api_key: resolved_key.cloned(),
    }
}

// the suffix goes on even when the base already ends in /embeddings
#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) if !base.is_empty() => format!("{base}/embeddings"),
        Some(_) | None => DEFAULT_URL.to_owned(),
    }
}

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut merged = Headers::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )]);
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        merged.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    merged.extend(headers.clone());
    merged
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    if let Some(dimensions) = non_default_params.get("dimensions") {
        optional_params.insert(String::from("dimensions"), dimensions.clone());
    }
}

// the input is JSON of any shape because LiteLLM keeps non-string items untouched
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Value, optional_params: &Object) -> Object {
    let items = match input {
        Value::Array(items) => items.clone(),
        single @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_)) => vec![single.clone()],
    };
    let carries_image = items
        .iter()
        .filter_map(Value::as_str)
        .any(is_base64_encoded);
    let normalized = if carries_image {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(|text| {
                let (part, value) = if is_base64_encoded(text) {
                    ("image", get_base64_str(text))
                } else {
                    ("text", text)
                };
                Value::Object(Object::from_iter([(
                    String::from(part),
                    Value::String(value.to_owned()),
                )]))
            })
            .collect()
    } else {
        items
    };
    let mut body = Object::from_iter([(String::from("model"), Value::String(model.to_owned()))]);
    body.extend(optional_params.clone());
    body.insert(String::from("input"), Value::Array(normalized));
    body
}

pub fn transform_embedding_response<F>(
    raw: &[u8],
    api_key: Option<&SecretString>,
    request_data: &Object,
    mut post_call: F,
) -> Result<EmbeddingResponse, EmbeddingError>
where
    F: FnMut(&PostCall<'_>),
{
    let mut response_json =
        serde_json::from_slice::<Value>(raw).map_err(|source| EmbeddingError::Decode { source })?;
    post_call(&PostCall {
        input: BuiltinInput,
        api_key,
        complete_input_dict: request_data,
        original_response: &response_json,
    });
    // LiteLLM's EmbeddingResponse sets object to "list" whatever the reply says
    if let Value::Object(fields) = &mut response_json {
        fields.insert(String::from("object"), Value::String(String::from("list")));
    }
    serde_json::from_value(response_json).map_err(|source| EmbeddingError::Decode { source })
}

mod tests;
