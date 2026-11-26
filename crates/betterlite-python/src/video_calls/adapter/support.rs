use std::error::Error as StdError;
use std::fmt::Display;
use std::io::Read as _;

use betterlite_core::api::video_remix_list_transport::error::ConfigFailure;
use betterlite_core::api::video_status_transport::error::ProviderFailure;
use betterlite_core::api::videos_http_transport::video_config::RequestFile;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::{MediaBody, MultipartPart};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::video_calls::convert::ConvertError;

pub(super) fn params_key(params: &IndexMap<String, Json>) -> Option<SecretString> {
    match params.get("api_key")? {
        Json::Text(text) => Some(SecretString::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// the multipart entry B525 built, with the caller's own file name when the payload knew one
pub(super) fn file_part(
    part: MultipartPart<'_>,
    name: Option<String>,
) -> Result<RequestFile, ConvertError> {
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

// a transform's failure as the status and remix/list traits report it: its text, with no status of its own
pub(crate) fn provider_failure<E>(error: E) -> ProviderFailure
where
    E: Display,
{
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
