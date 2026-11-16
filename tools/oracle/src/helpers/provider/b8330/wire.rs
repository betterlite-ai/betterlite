use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::data_cloud_dispatch::call::EmbeddingReply;
use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::fixture::Sent;

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

pub(super) fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::from_str(value),
        ) {
            map.insert(name, value);
        }
    }
    map
}

pub(super) fn body_request_view(sent: &Sent, names: &[&str]) -> Result<Value, Box<HelperError>> {
    let body: Value = serde_json::from_slice(&sent.body)?;
    Ok(json!({
        "url": sent.url,
        "headers": kept_headers(&sent.headers, names),
        "body": body,
    }))
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

pub(super) fn error_or_null(outcome: &Result<String, DispatchError>) -> Value {
    match outcome {
        Ok(_) => Value::Null,
        Err(error) => json!({"status": status_of(error)}),
    }
}

pub(super) fn reply_view(reply: &EmbeddingReply) -> Value {
    match reply {
        EmbeddingReply::Databricks(embedding) => {
            json!({"model": embedding.model, "count": embedding.data.len()})
        }
        EmbeddingReply::Snowflake(embedding) => {
            json!({"model": embedding.response.model, "count": embedding.response.data.len()})
        }
        EmbeddingReply::Oci(embedding) => {
            json!({"model": embedding.model, "count": embedding.data.len()})
        }
    }
}
