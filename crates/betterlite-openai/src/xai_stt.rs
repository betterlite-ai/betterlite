pub mod error;

use std::iter;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::audio_file_input::ProcessedAudioFile;
use crate::api::batch_records::WireDecodeLimits;
use crate::files::stream::flatten_headers;
use crate::xai_identity::{KeyOrder, get_api_base, get_api_key};
use error::XaiSttError;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["language"];
const EXCLUDED_PARAMS: [&str; 3] = ["model", "OPENAI_TRANSCRIPTION_PARAMS", "extra_body"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FormScalar {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FormValue {
    List(Vec<FormScalar>),
    Scalar(FormScalar),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FormText {
    Scalar(String),
    Repeated(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SttForm {
    pub fields: Vec<(String, FormValue)>,
    pub file: MultipartPart,
}

// LiteLLM's _XAISttResponse and _XAISttWord keep xAI's extra keys, so every reply field not modelled here rides along
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SttPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_language")]
    pub language: String,
    pub duration: Option<f64>,
    pub words: Option<Vec<SttWord>>,
    #[serde(flatten)]
    pub native_fields: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SttWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub speaker: Option<i64>,
    #[serde(flatten)]
    pub native_fields: Map<String, Value>,
}

fn default_language() -> String {
    "unknown".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionTask {
    Transcribe,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptionWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i64>,
}

// the complete native answer LiteLLM keeps for logging, plus the duration it copies out of it
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SttMetadata {
    #[serde(flatten)]
    pub native: SttPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_transcription_duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptionResponse {
    pub text: String,
    pub task: TranscriptionTask,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptionWord>>,
    #[serde(skip)]
    pub metadata: SttMetadata,
}

impl FormScalar {
    // Python's str() spelling of a list member: a bool capitalizes and a float keeps its repr
    #[must_use]
    pub fn member(&self) -> String {
        match self {
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => format!("{value:?}"),
            Self::Text(text) => text.clone(),
            Self::Null => "None".to_owned(),
        }
    }
}

impl FormValue {
    // a scalar bool reaches the form in lowercase, while list members keep Python's str() spelling
    #[must_use]
    pub fn form_text(&self) -> FormText {
        match self {
            Self::Scalar(FormScalar::Bool(flag)) => FormText::Scalar(flag.to_string()),
            Self::Scalar(scalar) => FormText::Scalar(scalar.member()),
            Self::List(items) => FormText::Repeated(items.iter().map(FormScalar::member).collect()),
        }
    }
}

#[must_use]
pub fn stt_url(api_base: Option<&str>, settings: &Settings) -> String {
    let base = get_api_base(api_base, settings);
    let trimmed = base.trim_end_matches('/');
    let normalized = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    format!("{normalized}/v1/stt")
}

pub fn authorization_headers(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<HeaderMap, XaiSttError> {
    let key = get_api_key(api_key, KeyOrder::Chat, settings).ok_or(XaiSttError::MissingApiKey)?;
    let value = HeaderValue::try_from(format!("Bearer {}", key.expose_secret()))
        .map_err(|source| XaiSttError::Authorization { source })?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

// LiteLLM's override keeps the caller's optional params and adds only the advertised language
pub fn map_openai_params(
    non_default_params: &[(String, Value)],
    optional_params: &mut Map<String, Value>,
) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// extra_body is flattened first and a direct option of the same name replaces it in place, None included
#[must_use]
pub fn transcription_form(
    model: &str,
    audio: &ProcessedAudioFile,
    extra_body: &IndexMap<String, FormValue>,
    direct: &IndexMap<String, FormValue>,
) -> SttForm {
    let mut flat: IndexMap<String, FormValue> = extra_body
        .clone()
        .into_iter()
        .chain(direct.clone())
        .collect();
    flat.retain(|name, value| {
        !EXCLUDED_PARAMS.contains(&name.as_str()) && *value != FormValue::Scalar(FormScalar::Null)
    });
    let model_field = (
        "model".to_owned(),
        FormValue::Scalar(FormScalar::Text(model.to_owned())),
    );
    SttForm {
        fields: iter::once(model_field).chain(flat).collect(),
        file: MultipartPart {
            name: "file".to_owned(),
            file_name: Some(audio.filename.clone()),
            content_type: Some(audio.content_type.clone()),
            data: audio.file_content.clone(),
        },
    }
}

// LiteLLM raises before parsing on a non-2xx reply, and a parse failure keeps the status it was handed
pub fn decode_xai_stt_reply(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
) -> Result<TranscriptionResponse, XaiSttError> {
    let headers = flatten_headers(&reply.headers);
    if reply.status >= 400 {
        return Err(XaiSttError::Status {
            status: reply.status,
            message: str::from_utf8(&reply.body).map_or_else(
                |_invalid| format!("{} bytes that are not UTF-8", reply.body.len()),
                str::to_owned,
            ),
            headers,
        });
    }
    if reply.body.len() > limits.max_json_bytes {
        return Err(XaiSttError::TooLarge {
            status: reply.status,
            headers,
            limit: limits.max_json_bytes,
        });
    }
    let payload: SttPayload =
        serde_json::from_slice(&reply.body).map_err(|source| XaiSttError::Decode {
            status: reply.status,
            headers,
            source,
        })?;
    Ok(transcription(payload))
}

// LiteLLM renames word.text to word, keeps a speaker of zero and stores the whole native answer as hidden params
fn transcription(payload: SttPayload) -> TranscriptionResponse {
    let words = payload.words.as_ref().map(|words| {
        words
            .iter()
            .map(|word| TranscriptionWord {
                word: word.text.clone(),
                start: word.start,
                end: word.end,
                speaker: word.speaker,
            })
            .collect()
    });
    TranscriptionResponse {
        text: payload.text.clone(),
        task: TranscriptionTask::Transcribe,
        language: payload.language.clone(),
        duration: payload.duration,
        words,
        metadata: SttMetadata {
            audio_transcription_duration: payload.duration,
            native: payload,
        },
    }
}

mod tests;
