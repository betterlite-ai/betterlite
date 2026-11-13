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
use super::config::TranscriptionFamily;
use super::error::TranscriptionError;
use crate::api::audio_file_input::ProcessedAudioFile;

// the order the OpenAI SDK writes its multipart body in, with the file part last
const SDK_FIELDS: [&str; 12] = [
    "chunking_strategy",
    "include",
    "keywords",
    "known_speaker_names",
    "known_speaker_references",
    "language",
    "languages",
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
    let mut options = optional_params.clone();
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

// The SDK's multipart encoding: a list repeats name[], a mapping nests name[key], and an empty text sends nothing.
fn field_parts(name: &str, value: &Json, parts: &mut Vec<MultipartPart>) {
    match value {
        Json::Null => {}
        Json::Text(text) => {
            if !text.is_empty() {
                parts.push(text_part(name, text));
            }
        }
        Json::Number(number) => parts.push(text_part(name, &number.to_string())),
        Json::Bool(flag) => parts.push(text_part(name, &flag.to_string())),
        Json::List(items) => {
            for item in items {
                field_parts(&format!("{name}[]"), item, parts);
            }
        }
        Json::Object(fields) => {
            for (key, item) in fields {
                field_parts(&format!("{name}[{key}]"), item, parts);
            }
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
        field_parts(name, value, &mut parts);
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
