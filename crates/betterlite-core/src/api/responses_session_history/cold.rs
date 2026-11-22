use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_session_history::decode::{SessionLogJsonKind, decode_session_log_json};
use crate::api::responses_session_history::error::SessionHistoryError;
use crate::api::responses_session_history::{ColdStorageReader, SessionHistory, SessionLog};

const TRUNCATED_MARKER: &str = "litellm_truncated";
const OBJECT_KEY: &str = "cold_storage_object_key";

// a missing, null or empty column and an empty cold-storage result all read as an empty request
pub async fn stored_request<Query, Cold, Sleep>(
    service: &SessionHistory<'_, Query, Cold, Sleep>,
    log: &SessionLog,
) -> Result<IndexMap<String, Json>, SessionHistoryError<Cold::Error>>
where
    Query: Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: Sync,
{
    let request = match &log.proxy_server_request {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Text(text)) => {
            if text.is_empty() {
                IndexMap::new()
            } else {
                decode_session_log_json(
                    text.as_bytes(),
                    SessionLogJsonKind::Request,
                    service.limits,
                )
                .map_err(SessionHistoryError::StoredRequest)?
                .unwrap_or_default()
            }
        }
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(SessionHistoryError::Column {
                column: "proxy_server_request",
            });
        }
    };
    let Some(cold) = service.cold_storage else {
        return Ok(request);
    };
    if !request_is_truncated(&request) {
        return Ok(request);
    }
    let Some(object_key) = cold_storage_key(log, service.limits) else {
        return Ok(request);
    };
    tracing::debug!("inside get_proxy_server_request_from_cold_storage_with_object_key...");
    let restored = cold
        .proxy_server_request(&object_key)
        .await
        .map_err(SessionHistoryError::ColdStorage)?;
    Ok(restored
        .filter(|fields| !fields.is_empty())
        .unwrap_or(request))
}

fn request_is_truncated(request: &IndexMap<String, Json>) -> bool {
    // LiteLLM searches str(request), so the marker in any key or nested string counts
    request.is_empty()
        || request
            .iter()
            .any(|(key, value)| key.contains(TRUNCATED_MARKER) || holds_truncation(value))
}

fn holds_truncation(value: &Json) -> bool {
    match value {
        Json::Text(text) => text.contains(TRUNCATED_MARKER),
        Json::List(items) => items.iter().any(holds_truncation),
        Json::Object(fields) => fields
            .iter()
            .any(|(key, item)| key.contains(TRUNCATED_MARKER) || holds_truncation(item)),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

// malformed metadata never fails the request: LiteLLM logs it and reads no key
fn cold_storage_key(log: &SessionLog, limits: ApiLimits) -> Option<String> {
    let decoded = match &log.metadata {
        Some(Json::Object(fields)) => return object_key(fields),
        Some(Json::Text(text)) => {
            decode_session_log_json(text.as_bytes(), SessionLogJsonKind::Metadata, limits)
        }
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => return None,
    };
    match decoded {
        Ok(fields) => fields.as_ref().and_then(object_key),
        Err(_malformed) => {
            tracing::debug!(
                "Failed to parse metadata from spend log to extract cold storage object key"
            );
            None
        }
    }
}

fn object_key(fields: &IndexMap<String, Json>) -> Option<String> {
    match fields.get(OBJECT_KEY) {
        Some(Json::Text(key)) if !key.is_empty() => Some(key.clone()),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => None,
    }
}
