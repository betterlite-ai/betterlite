#![expect(
    clippy::disallowed_types,
    reason = "a proxy request body is arbitrary JSON the client sends; only its top-level type is known"
)]

mod tests;

use crate::api::api_boundaries::ApiLimitError;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::str::Utf8Error;
use thiserror::Error;

pub type JsonObject = BTreeMap<String, serde_json::Value>;

const JSON_MEDIA_TYPE: &str = "application/json";
const FORM_MEDIA_TYPES: [&str; 2] = ["application/x-www-form-urlencoded", "multipart/form-data"];
const BINARY_MEDIA_TYPES: [&str; 2] = ["application/x-protobuf", "application/protobuf"];
const MULTIPART_MEDIA_TYPE: &str = "multipart/form-data";
const AUDIO_MEDIA_PREFIX: &str = "audio/";
const AZURE_SPEECH_ROUTE_PREFIX: &str = "/azure_speech/";
pub const OTLP_TRACE_METHOD: &str = "POST";
pub const OTLP_TRACE_PATH: &str = "/v1/traces";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTypeKind {
    Json,
    Form,
    Binary,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    Cached,
    Otlp,
    OpaqueAudio,
    Binary,
    Form,
    Json,
    RepairedJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedJson {
    Parsed { body: JsonObject, repaired: bool },
    // LiteLLM's lenient fallback decodes UTF-8 first, so unusable bytes leave auth with an empty body
    Unreadable(Utf8Error),
}

pub enum ParsedJson {
    Object(JsonObject),
    NotObject,
    Invalid,
}

// RFC 7231: the media type is everything before the first parameter, trimmed and lowercased
#[must_use]
pub fn normalize_media_type(content_type: &str) -> String {
    let media_type = content_type
        .split_once(';')
        .map_or(content_type, |(head, _parameters)| head);
    media_type.trim().to_lowercase()
}

#[must_use]
pub fn classify_media_type(content_type: &str) -> MediaTypeKind {
    let media_type = normalize_media_type(content_type);
    if media_type == JSON_MEDIA_TYPE {
        return MediaTypeKind::Json;
    }
    if FORM_MEDIA_TYPES.contains(&media_type.as_str()) {
        return MediaTypeKind::Form;
    }
    if BINARY_MEDIA_TYPES.contains(&media_type.as_str()) {
        return MediaTypeKind::Binary;
    }
    MediaTypeKind::Other
}

#[must_use]
pub fn parse_binary_body(body: &[u8]) -> JsonObject {
    match parse_json_bytes(body) {
        ParsedJson::Object(object) => object,
        ParsedJson::NotObject | ParsedJson::Invalid => JsonObject::new(),
    }
}

// Azure Speech forwards raw audio and multipart uploads byte for byte, so auth must not consume them
#[must_use]
pub fn is_opaque_audio_request(route: &str, content_type: &str) -> bool {
    let media_type = normalize_media_type(content_type);
    let audio = media_type.starts_with(AUDIO_MEDIA_PREFIX) || media_type == MULTIPART_MEDIA_TYPE;
    route.starts_with(AZURE_SPEECH_ROUTE_PREFIX) && audio
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedBody {
    accepted_keys: BTreeSet<String>,
    body: JsonObject,
}

impl ParsedBody {
    #[must_use]
    pub fn from_client_body(body: JsonObject) -> Self {
        let accepted_keys = body.keys().cloned().collect();
        Self {
            accepted_keys,
            body,
        }
    }

    // the accepted keys are a snapshot from when the body was cached, so params auth merged later stay hidden
    #[must_use]
    pub fn visible(&self) -> JsonObject {
        self.body
            .iter()
            .filter(|(key, _value)| self.accepted_keys.contains(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestBodyInput {
    pub method: String,
    pub route_path: String,
    pub headers: BTreeMap<String, String>,
    pub query_params: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub client_requested_model: Option<String>,
    pub cached_body: Option<ParsedBody>,
    pub form_fields: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBodyResult {
    pub branch: BodyKind,
    pub body: JsonObject,
    pub body_read: bool,
    pub parsed_body: Option<ParsedBody>,
}

impl RequestBodyResult {
    #[must_use]
    pub fn raw_json_body<'body>(&self, input: &'body RequestBodyInput) -> Option<&'body [u8]> {
        // a request without a content-type header has no media type to classify
        let content_type = input.headers.get("content-type").map_or("", String::as_str);
        if self.parsed_body.is_none() || classify_media_type(content_type) == MediaTypeKind::Form {
            return None;
        }
        Some(&input.body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenBody {
    pub request_data: JsonObject,
    pub cached_body: Option<ParsedBody>,
    pub raw_json_body: Option<Vec<u8>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestBodyError {
    #[error("the request body is not valid JSON")]
    InvalidJson,
    #[error("the request body must be a JSON object")]
    NotAnObject,
    #[error("the multipart form body was not decoded")]
    UndecodedForm,
    #[error("the form metadata field is not valid JSON")]
    InvalidFormMetadata,
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorFacts {
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
}

impl RequestBodyError {
    #[must_use]
    pub const fn facts(&self) -> ErrorFacts {
        match self {
            Self::InvalidJson
            | Self::NotAnObject
            | Self::UndecodedForm
            | Self::InvalidFormMetadata => ErrorFacts {
                status: 400,
                error_type: "invalid_request_error",
                param: "request_body",
                code: "400",
            },
            Self::Limit(error) => {
                let fields = error.fields();
                ErrorFacts {
                    status: fields.status,
                    error_type: fields.error_type,
                    param: fields.param,
                    code: fields.code,
                }
            }
        }
    }
}

#[must_use]
pub fn parse_json_bytes(body: &[u8]) -> ParsedJson {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(object)) => ParsedJson::Object(object.into_iter().collect()),
        Ok(_not_an_object) => ParsedJson::NotObject,
        Err(_invalid_json) => ParsedJson::Invalid,
    }
}

#[must_use]
pub fn parse_json_text(text: &str) -> ParsedJson {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(object)) => ParsedJson::Object(object.into_iter().collect()),
        Ok(_not_an_object) => ParsedJson::NotObject,
        Err(_invalid_json) => ParsedJson::Invalid,
    }
}

// the form decoder hands over its fields, so this only lifts them into JSON and resolves the metadata string
pub fn form_body(fields: &BTreeMap<String, String>) -> Result<JsonObject, RequestBodyError> {
    let mut body: JsonObject = fields
        .iter()
        .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
        .collect();
    if let Some(serde_json::Value::String(metadata)) = body.get("metadata").cloned() {
        let parsed = serde_json::from_str::<serde_json::Value>(&metadata)
            .map_err(|_invalid_metadata| RequestBodyError::InvalidFormMetadata)?;
        body.insert("metadata".to_owned(), parsed);
    }
    Ok(body)
}

#[must_use]
pub fn rewrite_request_model(
    request_data: &JsonObject,
    request: Option<&RequestBodyInput>,
    model: &str,
) -> RewrittenBody {
    let mut rewritten = request_data.clone();
    rewritten.insert(
        "model".to_owned(),
        serde_json::Value::String(model.to_owned()),
    );
    let Some(input) = request else {
        return RewrittenBody {
            request_data: rewritten,
            cached_body: None,
            raw_json_body: None,
        };
    };
    let cached_body = input.cached_body.as_ref().map_or_else(
        || ParsedBody::from_client_body(rewritten.clone()),
        |cached| {
            let mut body = cached.visible();
            body.insert(
                "model".to_owned(),
                serde_json::Value::String(model.to_owned()),
            );
            ParsedBody::from_client_body(body)
        },
    );
    // a parsed JSON object always serializes back, so this loses nothing
    let raw_json_body = serde_json::to_vec(&cached_body.visible()).ok();
    RewrittenBody {
        request_data: rewritten,
        cached_body: Some(cached_body),
        raw_json_body,
    }
}
