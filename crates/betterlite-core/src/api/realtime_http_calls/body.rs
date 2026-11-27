use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::realtime_contracts::records::{
    RealtimeExpiresAfter, RealtimeSessionConfig, TranscriptionSessionRequest,
};

use super::error::RecordError;
use super::{DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL};

// RealtimeSessionConfig.model_validate(session) if session else None: an empty mapping is falsy.
pub fn session(value: Option<&Json>) -> Result<Option<RealtimeSessionConfig>, RecordError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) if fields.is_empty() => Ok(None),
        Some(value) => decode(value, "session").map(Some),
    }
}

pub fn expires_after(value: Option<&Json>) -> Result<Option<RealtimeExpiresAfter>, RecordError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) if fields.is_empty() => Ok(None),
        Some(value) => decode(value, "expires_after").map(Some),
    }
}

// (req.session.model if req.session is not None else None) or req.model or "gpt-4o-realtime-preview"
pub fn client_secret_model(session: Option<&RealtimeSessionConfig>, model: Option<&str>) -> String {
    if let Some(nested) = session.and_then(|session| session.model.as_deref())
        && !nested.is_empty()
    {
        return nested.to_owned();
    }
    model
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_REALTIME_MODEL)
        .to_owned()
}

// model or "gpt-4o-realtime-preview"
pub fn calls_model(model: Option<&str>) -> String {
    model
        .filter(|model| !model.is_empty())
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
    if let Some(expires_after) = expires_after {
        body.insert(
            String::from("expires_after"),
            Json::Object(dumping(expires_after, "expires_after")?),
        );
    }
    if let Some(session) = session {
        let mut fields = dumping(session, "session")?;
        if session.model.is_some() {
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
    let fields: IndexMap<String, Json> = match transcription_session {
        None | Some(Json::Null) => IndexMap::new(),
        Some(value) => decode(value, "transcription_session")?,
    };
    if fields.contains_key("model") {
        return Err(RecordError::ConflictingModel);
    }
    let mut request: TranscriptionSessionRequest =
        decode(&Json::Object(fields), "transcription_session")?;
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
    body.remove("model");
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
    let Some(session) = session.filter(|value| !matches!(value, Json::Null)) else {
        return Ok(None);
    };
    let mut fields: IndexMap<String, Json> = decode(session, "session")?;
    if fields.contains_key("model") {
        fields.insert(String::from("model"), Json::Text(model.to_owned()));
    }
    Ok(Some(fields))
}

fn decode<T>(value: &Json, field: &'static str) -> Result<T, RecordError>
where
    T: DeserializeOwned,
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
    let encoded =
        serde_json::to_value(value).map_err(|source| RecordError::Invalid { field, source })?;
    let fields: IndexMap<String, Json> =
        serde_json::from_value(encoded).map_err(|source| RecordError::Invalid { field, source })?;
    Ok(fields
        .into_iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .collect())
}
