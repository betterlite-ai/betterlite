use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::gateway_provider_dispatch::error::DispatchError;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use super::input::Provider;

// the headers both sides build themselves; the HTTP client adds the rest, and the two providers case them differently
const KEPT_HEADERS: [&str; 4] = ["HTTP-Referer", "X-Title", "http-referer", "x-title"];

pub(super) fn request_view(
    url: &str,
    api_key: Option<&SecretString>,
    headers: Option<&BTreeMap<String, String>>,
) -> Value {
    let mut kept = Map::new();
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        kept.insert(
            String::from("authorization"),
            Value::String(format!("Bearer {}", key.expose_secret())),
        );
    }
    if let Some(given) = headers {
        for name in KEPT_HEADERS {
            if let Some(value) = given.get(name) {
                kept.insert(name.to_owned(), Value::String(value.clone()));
            }
        }
    }
    json!({"url": url, "headers": kept})
}

// the request body the shared handler builds from the arm's params, after it flattens extra_body
pub(super) fn params_view(
    optional_params: &Object,
    stream: Option<bool>,
    provider: Provider,
) -> Result<Value, Box<HelperError>> {
    let mut value = serde_json::to_value(optional_params)?;
    if let Value::Object(body) = &mut value {
        if let Some(Value::Object(extra)) = body.remove("extra_body") {
            for (key, entry) in extra {
                body.insert(key, entry);
            }
        }
        // OpenRouter's transform_request asks for the usage cost data whenever the request has no usage
        if provider == Provider::Openrouter && !body.contains_key("usage") {
            body.insert(String::from("usage"), json!({"include": true}));
        }
        if let Some(flag) = stream {
            body.insert(String::from("stream"), Value::Bool(flag));
        }
    }
    Ok(value)
}

// the status litellm's provider exception carries for this failure
pub(super) fn error_or_null<T>(outcome: &Result<T, DispatchError>) -> Value {
    match outcome {
        Ok(_) => Value::Null,
        Err(DispatchError::Handler(handler)) => json!({"status": handler.status_code()}),
        Err(DispatchError::ExtraBody { .. } | DispatchError::Encode { .. }) => {
            json!({"status": null})
        }
    }
}
