use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::transcription::request::TranscriptionData;
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use super::AzureTranscriptionError;
use super::client::RequestAuth;
use crate::transcription::encoding::quote_plus;

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
) -> Result<Vec<MultipartPart>, AzureTranscriptionError> {
    // create(**data) rejects a name its signature does not have, and an option cannot replace the audio file
    if let Some(unknown) = data
        .options
        .keys()
        .find(|name| !SDK_FIELDS.contains(&name.as_str()))
    {
        return Err(AzureTranscriptionError::Field {
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

// The SDK prefixes /deployments/{model} unless the base already names a deployment, then appends the endpoint.
fn request_url(
    base: &str,
    model: Option<&Json>,
    api_version: Option<&str>,
) -> Result<Url, AzureTranscriptionError> {
    let mut parsed =
        HttpxUrl::parse(base).map_err(|source| AzureTranscriptionError::Url { source })?;
    let base_query = parsed.query.take();
    parsed.fragment = None;
    let deployment = match model {
        Some(Json::Text(name)) => Some(name.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AzureTranscriptionError::Field {
                field: String::from("model"),
                problem: "is not a string",
            });
        }
        None | Some(Json::Null) => None,
    };
    let prefix = match deployment {
        Some(name) if !parsed.path.contains("/deployments") => format!("deployments/{name}/"),
        Some(_) | None => String::new(),
    };
    let mut composed = HttpxUrl::parse(&format!("{}{prefix}audio/transcriptions", parsed.href()))
        .map_err(|source| AzureTranscriptionError::Url { source })?;
    composed.query = merged_query(base_query.as_deref(), api_version);
    Url::parse(&composed.href()).map_err(|source| AzureTranscriptionError::BaseUrl { source })
}

// The SDK merges the base URL's query with api-version, which wins when both name it.
fn merged_query(base_query: Option<&str>, api_version: Option<&str>) -> Option<String> {
    if base_query.is_none() && api_version.is_none() {
        return None;
    }
    let mut pairs: Vec<String> = base_query
        .map(|query| {
            query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if let Some(version) = api_version {
        let encoded = format!("api-version={}", quote_plus(version));
        match pairs
            .iter_mut()
            .find(|pair| pair.split('=').next() == Some("api-version"))
        {
            Some(pair) => pair.clone_from(&encoded),
            None => pairs.push(encoded),
        }
    }
    Some(pairs.join("&"))
}

// what the selected client contributes to one request: its base, version, auth and deadline
pub struct OutboundTarget<'a> {
    pub base_url: &'a str,
    pub api_version: Option<&'a str>,
    pub auth: &'a RequestAuth,
    pub deadline: Duration,
}

pub fn outbound_request(
    target: &OutboundTarget<'_>,
    data: &TranscriptionData,
    file: &ProcessedAudioFile,
    boundary: String,
) -> Result<OutboundRequest, AzureTranscriptionError> {
    let url = request_url(target.base_url, data.model.as_ref(), target.api_version)?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    match target.auth {
        RequestAuth::ApiKey(key) => {
            let mut value = HeaderValue::try_from(key.expose_secret())
                .map_err(|source| AzureTranscriptionError::Header { source })?;
            value.set_sensitive(true);
            headers.insert("api-key", value);
        }
        RequestAuth::Bearer(token) => {
            let mut value = HeaderValue::try_from(format!("Bearer {}", token.expose_secret()))
                .map_err(|source| AzureTranscriptionError::Header { source })?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Multipart {
            boundary,
            parts: multipart_parts(data, file)?,
        },
        deadline: target.deadline,
    })
}
