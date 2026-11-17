use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use http::HeaderMap;
use serde_json::{Map, Value, json};

pub(super) fn kept_headers(headers: &HeaderMap, names: &[&str]) -> Value {
    let mut kept = Map::new();
    for name in names {
        if let Some(value) = headers.get(*name)
            && let Ok(text) = value.to_str()
        {
            kept.insert((*name).to_owned(), Value::String(text.to_owned()));
        }
    }
    Value::Object(kept)
}

pub(super) fn request_view(
    url: &str,
    headers: &HeaderMap,
    names: &[&str],
    body: Option<Value>,
) -> Value {
    let mut view = Map::new();
    view.insert(String::from("url"), Value::String(url.to_owned()));
    view.insert(String::from("headers"), kept_headers(headers, names));
    if let Some(body) = body {
        view.insert(String::from("body"), body);
    }
    Value::Object(view)
}

// the status LiteLLM's provider exception carries for this failure
fn status_of(error: &DispatchError) -> Option<u16> {
    match error {
        DispatchError::Handler(handler) => handler.status_code(),
        DispatchError::Upstream { status, .. } => Some(*status),
        DispatchError::Oci(failure) => Some(failure.status),
        DispatchError::SnowflakeException(failure) => Some(failure.status_code),
        DispatchError::Databricks(failure) => Some(failure.status_code),
        DispatchError::Snowflake(_)
        | DispatchError::SnowflakeEmbedding(_)
        | DispatchError::Environment { .. }
        | DispatchError::FieldType { .. }
        | DispatchError::Encode { .. }
        | DispatchError::Decode { .. }
        | DispatchError::Url { .. }
        | DispatchError::Headers { .. }
        | DispatchError::Stream { .. }
        | DispatchError::Transport(_) => None,
    }
}

pub(super) fn error_or_null<T>(outcome: &Result<T, DispatchError>) -> Value {
    match outcome {
        Ok(_) => Value::Null,
        Err(error) => json!({"status": status_of(error)}),
    }
}
