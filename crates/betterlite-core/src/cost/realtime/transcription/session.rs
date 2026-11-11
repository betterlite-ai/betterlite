use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::cost::realtime::Event;
use crate::cost::realtime::error::RealtimeCostError;

// Ports _get_transcription_model_name_from_results: the legacy object is blocked by a truthy nested one.
pub fn model_name(results: &[Event]) -> Result<Option<String>, RealtimeCostError> {
    for result in results {
        let Some(Json::Text(event_type)) = result.get("type") else {
            continue;
        };
        if !matches!(
            event_type.as_str(),
            "transcription_session.created"
                | "transcription_session.updated"
                | "session.created"
                | "session.updated"
        ) {
            continue;
        }
        if let Some(model) = session_model(session_object(result)?)? {
            return Ok(Some(model.to_owned()));
        }
    }
    Ok(None)
}

fn session_model(
    session: Option<&IndexMap<String, Json>>,
) -> Result<Option<&str>, RealtimeCostError> {
    let transcription = session_transcription(session)?;
    if let Some(fields) = transcription
        && let Some(model) = text_model(fields.get("model"))?
    {
        return Ok(Some(model));
    }
    let session_name = session.and_then(|fields| fields.get("model"));
    text_model(session_name)
}

fn session_transcription(
    session: Option<&IndexMap<String, Json>>,
) -> Result<Option<&IndexMap<String, Json>>, RealtimeCostError> {
    let Some(found) = session else {
        return Ok(None);
    };
    let Some(audio) = optional_object(found, "audio")? else {
        return optional_object(found, "input_audio_transcription");
    };
    let Some(input) = optional_object(audio, "input")? else {
        return optional_object(found, "input_audio_transcription");
    };
    let nested = optional_object(input, "transcription")?;
    if nested.is_some_and(|fields| !fields.is_empty()) {
        return Ok(nested);
    }
    optional_object(found, "input_audio_transcription")
}

fn session_object(event: &Event) -> Result<Option<&IndexMap<String, Json>>, RealtimeCostError> {
    match event.get("session") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(RealtimeCostError::Shape {
            field: "session",
            expected: "an object or null",
        }),
    }
}

fn optional_object<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'fields IndexMap<String, Json>>, RealtimeCostError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(nested)) => Ok(Some(nested)),
        Some(_) => Err(RealtimeCostError::Shape {
            field: name,
            expected: "an object or null",
        }),
    }
}

fn text_model(value: Option<&Json>) -> Result<Option<&str>, RealtimeCostError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(RealtimeCostError::Shape {
            field: "model",
            expected: "a string or null",
        }),
    }
}
