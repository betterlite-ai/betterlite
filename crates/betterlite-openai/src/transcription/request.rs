use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::client::TranscriptionClient;
use super::config::{TranscriptionFamily, map_openai_params};
use super::error::TranscriptionError;
use crate::api::audio_file_input::ProcessedAudioFile;

// the order the OpenAI SDK writes its multipart body in, with the file part last
const SDK_FIELDS: [&str; 7] = [
    "include",
    "language",
    "prompt",
    "response_format",
    "stream",
    "temperature",
    "timestamp_granularities",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionData {
    pub model: Option<Json>,
    pub options: IndexMap<String, Json>,
}

// OpenAIWhisperAudioTranscriptionConfig.transform_audio_transcription_request and its GPT variant.
pub fn prepare_data(
    family: Option<TranscriptionFamily>,
    model: &str,
    optional_params: &IndexMap<String, Json>,
) -> Result<TranscriptionData, TranscriptionError> {
    let mut options = family.map_or_else(
        || optional_params.clone(),
        |known| map_openai_params(known, optional_params),
    );
    if options.contains_key("file") {
        return Err(TranscriptionError::Field {
            field: String::from("file"),
            problem: "is already the audio file",
        });
    }
    let requested = options
        .shift_remove("model")
        .unwrap_or_else(|| Json::Text(model.to_owned()));
    // whisper asks for verbose_json so the duration arrives; an explicit value or null is kept
    if family == Some(TranscriptionFamily::Whisper) && !options.contains_key("response_format") {
        options.insert(
            String::from("response_format"),
            Json::Text(String::from("verbose_json")),
        );
    }
    Ok(TranscriptionData {
        model: Some(requested),
        options,
    })
}

// The data dict LiteLLM logs in pre_call and post_call, with the file object spelled as plain JSON.
#[must_use]
pub fn logged_data(data: &TranscriptionData, file: &ProcessedAudioFile) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.extend(
        data.model
            .iter()
            .map(|model| (String::from("model"), model.clone())),
    );
    body.insert(
        String::from("file"),
        Json::Object(IndexMap::from([
            (String::from("filename"), Json::Text(file.filename.clone())),
            (
                String::from("content_type"),
                Json::Text(file.content_type.clone()),
            ),
            (
                String::from("content"),
                Json::Text(BASE64.encode(&file.file_content)),
            ),
        ])),
    );
    body.extend(data.options.clone());
    body
}

fn text_part(name: &str, text: &str) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: None,
        content_type: None,
        data: text.as_bytes().to_vec(),
    }
}

fn field_parts(name: &str, value: &Json) -> Result<Vec<MultipartPart>, TranscriptionError> {
    let listed = matches!(name, "include" | "timestamp_granularities");
    let refuse = |problem| TranscriptionError::Field {
        field: name.to_owned(),
        problem,
    };
    match value {
        Json::Null => Ok(Vec::new()),
        Json::List(items) if listed => items
            .iter()
            .map(|item| match item {
                Json::Text(text) => Ok(text_part(&format!("{name}[]"), text)),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(refuse("is not a list of strings"))
                }
            })
            .collect(),
        Json::Text(text) if !listed => Ok(vec![text_part(name, text)]),
        Json::Number(number) if !listed => Ok(vec![text_part(name, &number.to_string())]),
        Json::Bool(flag) if !listed => Ok(vec![text_part(name, &flag.to_string())]),
        Json::Text(_) | Json::Number(_) | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(refuse(if listed {
                "is not a list of strings"
            } else {
                "is not a scalar"
            }))
        }
    }
}

fn multipart_parts(
    data: &TranscriptionData,
    file: &ProcessedAudioFile,
) -> Result<Vec<MultipartPart>, TranscriptionError> {
    if let Some(unknown) = data
        .options
        .keys()
        .find(|name| !SDK_FIELDS.contains(&name.as_str()))
    {
        return Err(TranscriptionError::Field {
            field: unknown.clone(),
            problem: "is not one the OpenAI SDK sends",
        });
    }
    let named = SDK_FIELDS
        .iter()
        .filter_map(|name| data.options.get(*name).map(|value| (*name, value)));
    let mut parts = Vec::new();
    for (name, value) in data.model.iter().map(|model| ("model", model)).chain(named) {
        parts.extend(field_parts(name, value)?);
    }
    parts.push(MultipartPart {
        name: String::from("file"),
        file_name: Some(file.filename.clone()),
        content_type: Some(file.content_type.clone()),
        data: file.file_content.clone(),
    });
    Ok(parts)
}

pub fn outbound_request(
    client: &TranscriptionClient,
    data: &TranscriptionData,
    file: &ProcessedAudioFile,
    boundary: String,
) -> Result<OutboundRequest, TranscriptionError> {
    let url = client
        .base_url
        .join("audio/transcriptions")
        .map_err(|source| TranscriptionError::BaseUrl { source })?;
    let mut authorization =
        HeaderValue::try_from(format!("Bearer {}", client.api_key.expose_secret()))
            .map_err(|source| TranscriptionError::Header { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(AUTHORIZATION, authorization);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Multipart {
            boundary,
            parts: multipart_parts(data, file)?,
        },
        deadline: client.timeout,
    })
}
