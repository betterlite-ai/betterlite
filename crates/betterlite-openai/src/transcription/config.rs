use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::decode::text_field;
use super::error::TranscriptionError;
use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::{Headers, Object};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionFamily {
    Whisper,
    Gpt,
}

#[must_use]
pub const fn supported_openai_params(family: TranscriptionFamily) -> &'static [&'static str] {
    match family {
        TranscriptionFamily::Whisper => &[
            "language",
            "prompt",
            "response_format",
            "temperature",
            "timestamp_granularities",
        ],
        TranscriptionFamily::Gpt => &[
            "language",
            "prompt",
            "response_format",
            "temperature",
            "include",
        ],
    }
}

#[must_use]
pub fn map_openai_params(
    family: TranscriptionFamily,
    non_default_params: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let supported = supported_openai_params(family);
    non_default_params
        .iter()
        .filter(|(name, _value)| supported.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// LiteLLM writes "Bearer None" when the explicit key and OPENAI_API_KEY are both unset.
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment: Option<&SecretString>,
) -> Headers {
    let authorization = api_key.or(environment).map_or_else(
        || String::from("Bearer None"),
        |key| format!("Bearer {}", key.expose_secret()),
    );
    let mut updated = headers.clone();
    updated.insert(String::from("Authorization"), authorization);
    updated
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.unwrap_or_default().trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/audio/transcriptions")
    } else {
        format!("{base}/v1/audio/transcriptions")
    }
}

// transform_audio_transcription_response: TranscriptionResponse(**json) takes the text keyword only.
pub fn transform_response(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<TranscriptionResponse, TranscriptionError> {
    let fields = match serde_json::from_slice::<Json>(body) {
        Ok(Json::Object(fields)) => fields,
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(TranscriptionError::NotObject);
        }
        Err(source) => {
            if content_type
                .is_some_and(|kind| kind.to_ascii_lowercase().contains("application/json"))
            {
                return Err(TranscriptionError::Json { source });
            }
            let text = from_utf8(body).map_err(|utf8| TranscriptionError::Utf8 { source: utf8 })?;
            return Ok(TranscriptionResponse {
                text: Some(text.to_owned()),
                extra: Object::new(),
            });
        }
    };
    if !fields.contains_key("text") && !fields.contains_key("usage") {
        return Err(TranscriptionError::NoResponseFields);
    }
    if let Some(field) = fields.keys().find(|name| *name != "text") {
        return Err(TranscriptionError::Field {
            field: field.clone(),
            problem: "is not a TranscriptionResponse argument",
        });
    }
    Ok(TranscriptionResponse {
        text: text_field(&fields)?,
        extra: Object::new(),
    })
}
