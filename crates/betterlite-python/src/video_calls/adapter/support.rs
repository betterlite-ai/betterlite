use std::error::Error as StdError;
use std::io::Read as _;

use betterlite_core::api::video_remix_list_transport::error::ConfigFailure;
use betterlite_core::api::video_status_transport::error::ProviderFailure;
use betterlite_core::api::videos_http_transport::video_config::RequestFile;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::{MediaBody, MultipartPart};
use betterlite_openai::videos::error::VideoError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::AdapterError;

// B525's wire types and this crate's params are the same JSON, so each conversion is one round trip
pub(super) fn json<T, V>(value: &V) -> Result<T, AdapterError>
where
    T: DeserializeOwned,
    V: Serialize,
{
    let encoded = serde_json::to_value(value)?;
    serde_json::from_value(encoded).map_err(AdapterError::from)
}

pub(super) fn params_key(params: &IndexMap<String, Json>) -> Option<SecretString> {
    match params.get("api_key")? {
        Json::Text(text) => Some(SecretString::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn file_part(
    part: MultipartPart<'_>,
    name: Option<String>,
) -> Result<RequestFile, AdapterError> {
    let MultipartPart {
        field,
        filename,
        content_type,
        body,
    } = part;
    let content = match body {
        MediaBody::Bytes(bytes) => bytes.to_vec(),
        MediaBody::Text(text) => text.as_bytes().to_vec(),
        MediaBody::Stream { mut reader, .. } => {
            let mut read = Vec::new();
            reader.read_to_end(&mut read)?;
            read
        }
    };
    Ok(RequestFile {
        field,
        filename: Some(name.unwrap_or(filename)),
        content,
        content_type: Some(content_type.to_owned()),
    })
}

// VideoError as the status and remix/list transforms report it: its text, with no status of its own
pub(crate) fn provider_failure(error: VideoError) -> ProviderFailure {
    ProviderFailure {
        message: error.to_string(),
        status_code: None,
        headers: None,
    }
}

pub(crate) fn config_failure<E>(error: E) -> ConfigFailure
where
    E: StdError + Send + Sync + 'static,
{
    ConfigFailure {
        source: Box::new(error),
    }
}
