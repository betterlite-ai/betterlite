use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Serialize, de::DeserializeOwned};

use crate::api::realtime_contracts::records::{
    RealtimeExpiresAfter, RealtimeSessionConfig, TranscriptionSessionRequest,
};

use super::error::RecordError;
use super::{DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL};

// A record decoded from the caller's JSON: an absent, null or empty mapping is None (pydantic's falsy check).
pub(super) fn optional<To>(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<To>, RecordError>
where
    To: DeserializeOwned,
{
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) if fields.is_empty() => Ok(None),
        Some(other) => converted(other, field).map(Some),
    }
}

#[must_use]
pub fn model_or_default(model: Option<&str>) -> String {
    model
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_REALTIME_MODEL)
        .to_owned()
}

// RealtimeClientSecretRequest.model_dump(exclude_none=True, exclude={"model"}) and the session model rewrite.
pub fn client_secret(
    session: Option<&RealtimeSessionConfig>,
    expires_after: Option<&RealtimeExpiresAfter>,
    model: &str,
) -> Result<IndexMap<String, Json>, RecordError> {
    let mut body = IndexMap::new();
    if let Some(expiry) = expires_after {
        body.insert(
            String::from("expires_after"),
            Json::Object(dumping(expiry, "expires_after")?),
        );
    }
    if let Some(config) = session {
        let mut fields = dumping(config, "session")?;
        if config.model.is_some() {
            fields.insert(String::from("model"), Json::Text(model.to_owned()));
        }
        body.insert(String::from("session"), Json::Object(fields));
    }
    Ok(body)
}

// RealtimeTranscriptionSessionRequest(model=model, **(transcription_session or {})) and its resolved routing model.
pub fn transcription(
    transcription_session: Option<&Json>,
    model: Option<&str>,
) -> Result<(TranscriptionSessionRequest, String), RecordError> {
    let fields: IndexMap<String, Json> =
        optional(transcription_session, "transcription_session")?.unwrap_or_default();
    if fields.contains_key("model") {
        return Err(RecordError::ConflictingModel);
    }
    let mut request: TranscriptionSessionRequest =
        converted(&Json::Object(fields), "transcription_session")?;
    request.model = model.map(str::to_owned);
    let model_name = request
        .resolved_model()
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_TRANSCRIPTION_MODEL)
        .to_owned();
    Ok((request, model_name))
}

pub fn transcription_body(
    request: &TranscriptionSessionRequest,
    model: &str,
) -> Result<IndexMap<String, Json>, RecordError> {
    let mut body = dumping(request, "transcription_session")?;
    body.shift_remove("model");
    if let Some(Json::Object(transcription)) = body.get_mut("input_audio_transcription") {
        transcription.insert(String::from("model"), Json::Text(model.to_owned()));
    }
    Ok(body)
}

// _with_resolved_session_model: only a session that already names a model has it rewritten, a null included.
pub fn calls_session(
    session: Option<&Json>,
    model: &str,
) -> Result<Option<IndexMap<String, Json>>, RecordError> {
    let Some(source) = session.filter(|value| !matches!(value, Json::Null)) else {
        return Ok(None);
    };
    let mut fields: IndexMap<String, Json> = converted(source, "session")?;
    if fields.contains_key("model") {
        fields.insert(String::from("model"), Json::Text(model.to_owned()));
    }
    Ok(Some(fields))
}

// The pydantic record and the wire JSON both come from one serde round trip.
fn converted<From, To>(value: &From, field: &'static str) -> Result<To, RecordError>
where
    From: Serialize + ?Sized,
    To: DeserializeOwned,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| RecordError::Invalid { field, source })?;
    serde_json::from_value(encoded).map_err(|source| RecordError::Invalid { field, source })
}

// model_dump(exclude_none=True): a null declared field and a null extra are dropped, a nested plain mapping is not walked.
fn dumping<T>(value: &T, field: &'static str) -> Result<IndexMap<String, Json>, RecordError>
where
    T: Serialize + ?Sized,
{
    let fields: IndexMap<String, Json> = converted(value, field)?;
    Ok(fields
        .into_iter()
        .filter(|(_, entry)| !matches!(entry, Json::Null))
        .collect())
}
