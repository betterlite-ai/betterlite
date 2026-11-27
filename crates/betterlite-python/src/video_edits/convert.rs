use std::time::TryFromFloatSecsError;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

pub(crate) mod fields;
pub(crate) mod requests;
mod tests;

use self::fields::failed;

const VIDEO: &str = "video";
const DEFAULT_NAME: &str = "file";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct VideoEditWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
}

// The caller's video under the name the payload gave it, and the bytes themselves
#[derive(Debug, Clone)]
pub(crate) struct VideoFile {
    pub(crate) name: Option<String>,
    pub(crate) media: SuppliedMedia,
}

#[derive(Debug, Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{name} must be {expected}")]
    Field {
        name: String,
        expected: &'static str,
    },
    #[error("the timeout is not a number of seconds: {source}")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Files(#[from] CallFilesError),
}

// the wire and the one video file the call carries, taken out of the payload's blobs before any field reads it
pub(crate) fn parse(
    payload: &str,
    blobs: &mut [Vec<u8>],
) -> Result<(VideoEditWire, Option<VideoFile>), ConvertError> {
    let wire: VideoEditWire =
        serde_json::from_str(payload).map_err(|source| ConvertError::Json { source })?;
    let files = files_from_payload(&wire.arguments, blobs)?;
    let video = files
        .iter()
        .find(|(name, _file)| name == VIDEO)
        .map(|(_name, file)| video_file(file));
    Ok((wire, video))
}

fn video_file(file: &CallFile) -> VideoFile {
    VideoFile {
        name: Some(file.name.clone()).filter(|name| !name.is_empty() && name != DEFAULT_NAME),
        media: SuppliedMedia::Bytes(file.bytes.clone()),
    }
}

// the video argument itself: a marker the payload extracted, a plain string LiteLLM sends as file content, or nothing
pub(crate) fn video_media(
    arguments: &RequestMap,
    file: Option<&VideoFile>,
) -> Result<Option<VideoFile>, ConvertError> {
    if let Some(found) = file {
        return Ok(Some(found.clone()));
    }
    match arguments.get(VIDEO) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(VideoFile {
            name: None,
            media: SuppliedMedia::Text(text.clone()),
        })),
        Some(_) => Err(failed(VIDEO, "a file")),
    }
}

// the loose keywords LiteLLM's **kwargs takes, minus the named arguments the call hands over on their own
fn loose(arguments: &RequestMap, named: &[&str]) -> IndexMap<String, Json> {
    arguments
        .iter()
        .filter(|(name, _value)| !named.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

const CREATE_NAMED: [&str; 11] = [
    "name",
    "video",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "mock_response",
    "litellm_call_id",
    "litellm_logging_obj",
    "async_call",
];

const GET_NAMED: [&str; 10] = [
    "character_id",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "mock_response",
    "litellm_call_id",
    "litellm_logging_obj",
    "async_call",
];

const EDIT_NAMED: [&str; 9] = [
    "video_id",
    "prompt",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "video",
    "async_call",
];

const EXTENSION_NAMED: [&str; 9] = [
    "video_id",
    "prompt",
    "seconds",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "async_call",
];
