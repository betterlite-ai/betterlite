use std::io::Error as IoError;
use std::time::TryFromFloatSecsError;

use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_remix_call::VideoRemixCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::videos::error::VideoError;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

pub(crate) mod fields;
mod requests;
mod tests;

use self::fields::failed;

const INPUT_REFERENCE: &str = "input_reference";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct VideoCallWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) drop_params: Option<bool>,
}

// A file the caller handed as input_reference: its multipart name, "file" or the basename, and its bytes
#[derive(Debug, Clone)]
pub(crate) struct Reference {
    pub(crate) name: Option<String>,
    pub(crate) content: Vec<u8>,
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
    #[error(transparent)]
    Video(#[from] VideoError),
    #[error("the video reference could not be read: {source}")]
    Io {
        #[from]
        source: IoError,
    },
    #[error("the native call asked for a different video operation")]
    Kind,
}

// which of the ten functions the payload belongs to: each names one request type of the core call layer
#[derive(Debug, Clone, Copy)]
pub(crate) enum Operation {
    Generate,
    Content,
    Remix,
    List,
    Status,
}

// the one request the operation asked for, so one parser serves all five payload shapes
#[derive(Default)]
pub(crate) struct Requests {
    pub(crate) generation: Option<VideoGenerationRequest>,
    pub(crate) content: Option<VideoContentRequest>,
    pub(crate) remix: Option<VideoRemixCall>,
    pub(crate) list: Option<VideoListCall>,
    pub(crate) status: Option<VideoStatusRequest>,
}

pub(crate) fn parse(
    payload: &str,
    blobs: &mut [Vec<u8>],
) -> Result<(VideoCallWire, Option<Reference>), ConvertError> {
    let wire: VideoCallWire =
        serde_json::from_str(payload).map_err(|source| ConvertError::Json { source })?;
    let files = files_from_payload(&wire.arguments, blobs)?;
    let reference = files
        .iter()
        .find(|(name, _file)| name == INPUT_REFERENCE)
        .map(|(_name, file)| reference(file));
    Ok((wire, reference))
}

fn reference(file: &CallFile) -> Reference {
    Reference {
        name: Some(file.name.clone()).filter(|name| !name.is_empty() && name != "file"),
        content: file.bytes.clone(),
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

// timeout or REQUEST_TIMEOUT: an explicit null or zero takes the request timeout further down
pub(crate) fn drop_params(wire: &VideoCallWire) -> Result<bool, ConvertError> {
    let given = match wire.arguments.get("drop_params") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(value)) => *value,
        Some(_) => return Err(failed("drop_params", "a boolean")),
    };
    Ok(given || wire.settings.drop_params.unwrap_or(false))
}

const GENERATION_NAMED: [&str; 11] = [
    "prompt",
    "model",
    "input_reference",
    "seconds",
    "size",
    "user",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
];

const CONTENT_NAMED: [&str; 7] = [
    "video_id",
    "timeout",
    "custom_llm_provider",
    "variant",
    "extra_headers",
    "extra_query",
    "extra_body",
];

const REMIX_NAMED: [&str; 8] = [
    "video_id",
    "prompt",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "api_key",
];

const LIST_NAMED: [&str; 11] = [
    "after",
    "limit",
    "order",
    "api_key",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
    "video_id",
    "prompt",
];

const STATUS_NAMED: [&str; 6] = [
    "video_id",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
];

pub(crate) fn request(
    operation: Operation,
    wire: &VideoCallWire,
    media: Option<Reference>,
) -> Result<Requests, ConvertError> {
    let arguments = &wire.arguments;
    let mut requests = Requests::default();
    match operation {
        Operation::Generate => requests.generation = Some(requests::generation(arguments, media)?),
        Operation::Content => requests.content = Some(requests::content(arguments)?),
        Operation::Remix => requests.remix = Some(requests::remix(arguments)?),
        Operation::List => requests.list = Some(requests::list(arguments)?),
        Operation::Status => requests.status = Some(requests::status(arguments)?),
    }
    Ok(requests)
}
