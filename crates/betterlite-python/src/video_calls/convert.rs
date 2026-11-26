use std::io::Error as IoError;
use std::time::TryFromFloatSecsError;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
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
mod tests;

use self::fields::{
    INTEGER, duration, failed, meta, number, params, required_text, seconds, text, texts,
};

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
        Operation::Generate => {
            let mut generation = VideoGenerationRequest::new(required_text(arguments, "prompt")?);
            generation.model = text(arguments, "model")?;
            generation.seconds = seconds(arguments, "seconds")?;
            generation.size = text(arguments, "size")?;
            generation.user = text(arguments, "user")?;
            generation.timeout = number(arguments, "timeout")?;
            generation.custom_llm_provider = text(arguments, "custom_llm_provider")?;
            generation.extra_headers = texts(arguments, "extra_headers")?;
            generation.extra_query = params(arguments, "extra_query")?;
            generation.extra_body = params(arguments, "extra_body")?;
            generation.input_reference = media
                .as_ref()
                .map(|reference| SuppliedMedia::Bytes(reference.content.clone()));
            generation.kwargs = loose(arguments, &GENERATION_NAMED);
            requests.generation = Some(generation);
        }
        Operation::Content => {
            requests.content = Some(VideoContentRequest {
                video_id: required_text(arguments, "video_id")?,
                timeout: duration(number(arguments, "timeout")?)?,
                custom_llm_provider: text(arguments, "custom_llm_provider")?,
                variant: text(arguments, "variant")?,
                extra_headers: texts(arguments, "extra_headers")?,
                extra_query: params(arguments, "extra_query")?,
                extra_body: params(arguments, "extra_body")?,
                kwargs: loose(arguments, &CONTENT_NAMED),
            });
        }
        Operation::Remix => {
            requests.remix = Some(VideoRemixCall {
                video_id: required_text(arguments, "video_id")?,
                prompt: required_text(arguments, "prompt")?,
                custom_llm_provider: text(arguments, "custom_llm_provider")?,
                api_key: text(arguments, "api_key")?.map(SecretString::from),
                api_base: text(arguments, "api_base")?,
                api_version: text(arguments, "api_version")?,
                extra_headers: texts(arguments, "extra_headers")?,
                extra_query: params(arguments, "extra_query")?,
                extra_body: params(arguments, "extra_body")?,
                mock_response: arguments.get("mock_response").cloned(),
                litellm_metadata: meta(arguments, "litellm_metadata")?,
                litellm_call_id: text(arguments, "litellm_call_id")?,
                user: text(arguments, "user")?,
                timeout: number(arguments, "timeout")?,
                extra: loose(arguments, &REMIX_NAMED),
            });
        }
        Operation::List => {
            requests.list = Some(VideoListCall {
                after: text(arguments, "after")?,
                limit: match arguments.get("limit") {
                    None | Some(Json::Null) => None,
                    Some(Json::Number(value)) => {
                        Some(value.as_i64().ok_or_else(|| failed("limit", INTEGER))?)
                    }
                    Some(_) => return Err(failed("limit", INTEGER)),
                },
                order: text(arguments, "order")?,
                custom_llm_provider: text(arguments, "custom_llm_provider")?,
                api_key: text(arguments, "api_key")?.map(SecretString::from),
                api_base: text(arguments, "api_base")?,
                api_version: text(arguments, "api_version")?,
                extra_headers: texts(arguments, "extra_headers")?,
                extra_query: params(arguments, "extra_query")?,
                extra_body: params(arguments, "extra_body")?,
                mock_response: arguments.get("mock_response").cloned(),
                litellm_metadata: meta(arguments, "litellm_metadata")?,
                litellm_call_id: text(arguments, "litellm_call_id")?,
                user: text(arguments, "user")?,
                timeout: number(arguments, "timeout")?,
                extra: loose(arguments, &LIST_NAMED),
            });
        }
        Operation::Status => {
            requests.status = Some(VideoStatusRequest {
                video_id: required_text(arguments, "video_id")?,
                timeout: Some(duration(number(arguments, "timeout")?)?.unwrap_or_default()),
                custom_llm_provider: text(arguments, "custom_llm_provider")?,
                extra_headers: texts(arguments, "extra_headers")?,
                extra_query: params(arguments, "extra_query")?,
                extra_body: params(arguments, "extra_body")?,
                kwargs: loose(arguments, &STATUS_NAMED),
            });
        }
    }
    Ok(requests)
}
