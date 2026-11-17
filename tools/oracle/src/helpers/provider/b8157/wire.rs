use betterlite_provider::self_hosted_dispatch::error::DispatchError;
use http::HeaderMap;
use serde_json::{Map, Value, json};

// only the header both sides prepare themselves; the HTTP client adds the rest
pub(super) fn kept_authorization(headers: &HeaderMap) -> Value {
    let mut kept = Map::new();
    if let Some(value) = headers.get("authorization")
        && let Ok(text) = value.to_str()
    {
        kept.insert(
            String::from("authorization"),
            Value::String(text.to_owned()),
        );
    }
    Value::Object(kept)
}

// the status LiteLLM's provider exception carries for this failure
pub(super) fn error_or_null<T>(outcome: &Result<T, DispatchError>) -> Value {
    match outcome {
        Ok(_) => Value::Null,
        Err(error) => json!({"status": error.status()}),
    }
}
