use betterlite_logging::spend_bodies::classifier::{
    classifier_audit_fields, decode_classifier_snapshot,
};
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_logging::spend_sanitize::truncate::TRUNCATED_PAYLOAD_FIELD;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::cold_storage::ColdStorageHandler;
use super::convert::column_json;
use super::metadata::cold_storage_object_key;
use crate::spend::visibility::store::PayloadRow;

const EMPTY_VALUES: [&str; 4] = ["", "{}", "[]", "null"];
const MESSAGES_FIELD: &str = "messages";
const RESPONSE_FIELD: &str = "response";
const REQUEST_FIELD: &str = "proxy_server_request";

// RequestResponsePayload._asdict(): a missing field is null
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestResponsePayload {
    pub messages: Option<Json>,
    pub response: Option<Json>,
    pub proxy_server_request: Option<Json>,
}

// _spend_log_field_has_content: a number or a boolean counts, even zero or false
#[must_use]
pub fn has_content(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        // str.strip also drops the four ASCII separators Rust does not count as whitespace
        Some(Json::Text(text)) => !EMPTY_VALUES.contains(&text.trim_matches(|letter: char| {
            letter.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&letter)
        })),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Bool(_) | Json::Number(_)) => true,
    }
}

// LiteLLM searches str(proxy_server_request), so the marker in any key or nested string counts
#[must_use]
pub fn holds_truncation(value: &Json) -> bool {
    match value {
        Json::Text(text) => text.contains(TRUNCATED_PAYLOAD_FIELD),
        Json::List(items) => items.iter().any(holds_truncation),
        Json::Object(fields) => fields
            .iter()
            .any(|(key, item)| key.contains(TRUNCATED_PAYLOAD_FIELD) || holds_truncation(item)),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

// classifier_input_snapshot over a stored value: JSON text must decode to an object, an object is taken as is
#[must_use]
pub fn request_snapshot(value: &Json) -> Option<Fields> {
    match value {
        Json::Text(text) => match decode_classifier_snapshot(text.as_bytes()) {
            Some(Json::Object(fields)) => Some(fields),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        },
        Json::Object(fields) => Some(fields.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}

// _resolve_request_response_payload: the row's own content wins unless its classifier audit was truncated
pub async fn resolve_request_response_payload(
    row: &PayloadRow,
    cold_storage: &ColdStorageHandler,
) -> RequestResponsePayload {
    let stored = RequestResponsePayload {
        messages: row.messages.as_ref().map(column_json),
        response: row.response.as_ref().map(column_json),
        proxy_server_request: row.proxy_server_request.as_ref().map(column_json),
    };
    let stored_request = stored
        .proxy_server_request
        .as_ref()
        .and_then(request_snapshot);
    let truncated_audit = stored_request
        .as_ref()
        .is_some_and(|snapshot| !classifier_audit_fields(snapshot).is_empty())
        && stored
            .proxy_server_request
            .as_ref()
            .is_some_and(holds_truncation);
    let stored_content = has_content(stored.messages.as_ref())
        || has_content(stored.response.as_ref())
        || has_content(stored.proxy_server_request.as_ref());
    if !truncated_audit && stored_content {
        return stored;
    }
    let Some(payload) = cold_payload(row, cold_storage).await else {
        return stored;
    };
    if truncated_audit {
        return recovered_audit(stored, &payload, stored_request);
    }
    let cold_audit = classifier_audit_fields(&payload);
    RequestResponsePayload {
        messages: payload.get(MESSAGES_FIELD).cloned(),
        response: payload.get(RESPONSE_FIELD).cloned(),
        proxy_server_request: if cold_audit.is_empty() {
            payload.get(REQUEST_FIELD).cloned()
        } else {
            Some(audited_request(&payload, stored_request, cold_audit))
        },
    }
}

// the object the row's metadata names; a failed read warns and leaves the row's values in place
async fn cold_payload(row: &PayloadRow, cold_storage: &ColdStorageHandler) -> Option<Fields> {
    let object_key = cold_storage_object_key(row.metadata.as_ref())?;
    match cold_storage.proxy_server_request(&object_key).await {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Failed to fetch cold storage payload for key {object_key}; falling back to DB values"
            );
            None
        }
    }
}

// a truncated audit keeps the row's messages and response and takes only the cold audit, when there is one
fn recovered_audit(
    stored: RequestResponsePayload,
    payload: &Fields,
    stored_request: Option<Fields>,
) -> RequestResponsePayload {
    let cold_audit = classifier_audit_fields(payload);
    if cold_audit.is_empty() {
        return stored;
    }
    RequestResponsePayload {
        messages: stored.messages,
        response: stored.response,
        proxy_server_request: Some(audited_request(payload, stored_request, cold_audit)),
    }
}

// the cold request snapshot, else the stored one, with the cold audit fields laid over it
fn audited_request(payload: &Fields, stored_request: Option<Fields>, cold_audit: Fields) -> Json {
    let mut request = payload
        .get(REQUEST_FIELD)
        .and_then(request_snapshot)
        .filter(|snapshot| !snapshot.is_empty())
        .or_else(|| stored_request.filter(|snapshot| !snapshot.is_empty()))
        .unwrap_or_default();
    request.extend(cold_audit);
    Json::Object(request)
}
