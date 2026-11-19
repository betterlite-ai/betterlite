use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::hosted_platform_dispatch::error::DispatchError;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

// the two headers both sides prepare themselves; the HTTP client adds the rest
const KEPT_HEADERS: [&str; 1] = ["x-scope"];

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
    let given = headers.cloned().unwrap_or_default();
    for name in KEPT_HEADERS {
        if let Some(value) = given.get(name) {
            kept.insert(name.to_owned(), Value::String(value.clone()));
        }
    }
    json!({"url": url, "headers": kept})
}

// the request body the shared handler builds from the arm's params, minus the fields the case fixes
pub(super) fn params_view(optional_params: &Object) -> Result<Value, Box<HelperError>> {
    let mut value = serde_json::to_value(optional_params)?;
    if let Value::Object(body) = &mut value
        && let Some(Value::Object(extra)) = body.remove("extra_body")
    {
        for (key, entry) in extra {
            body.insert(key, entry);
        }
    }
    Ok(value)
}

// the status litellm's provider exception carries for this failure
pub(super) fn error_or_null<T>(outcome: &Result<T, DispatchError>) -> Value {
    match outcome {
        Ok(_) => Value::Null,
        Err(error) => json!({"status": status_of(error)}),
    }
}

fn status_of(error: &DispatchError) -> Option<u16> {
    match error {
        DispatchError::Handler(handler) => handler.status_code(),
        DispatchError::NotBuilt { .. }
        | DispatchError::Cloudflare(_)
        | DispatchError::Encode { .. } => None,
    }
}
