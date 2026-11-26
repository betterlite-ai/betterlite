use std::time::{Duration, TryFromFloatSecsError};

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_remix_call::VideoRemixCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use self::fields::{
    duration, integer, metadata, number, params, required_text, seconds_value, secret, text,
    text_map,
};

pub(crate) mod fields;
mod tests;

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
pub(crate) fn loose(arguments: &RequestMap, named: &[&str]) -> IndexMap<String, Json> {
    arguments
        .iter()
        .filter(|(name, _value)| !named.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
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

pub(crate) fn generation_request(
    wire: &VideoCallWire,
    media: Option<Reference>,
) -> Result<(VideoGenerationRequest, Option<Reference>), ConvertError> {
    let arguments = &wire.arguments;
    let mut request = VideoGenerationRequest::new(required_text(arguments, "prompt")?);
    request.model = text(arguments, "model")?;
    request.seconds = seconds_value(arguments, "seconds")?;
    request.size = text(arguments, "size")?;
    request.user = text(arguments, "user")?;
    request.timeout = number(arguments, "timeout")?;
    request.custom_llm_provider = text(arguments, "custom_llm_provider")?;
    request.extra_headers = text_map(arguments, "extra_headers")?;
    request.extra_query = params(arguments, "extra_query")?;
    request.extra_body = params(arguments, "extra_body")?;
    request.input_reference = media
        .as_ref()
        .map(|reference| SuppliedMedia::Bytes(reference.content.clone()));
    request.kwargs = loose(arguments, &GENERATION_NAMED);
    Ok((request, media))
}

const CONTENT_NAMED: [&str; 7] = [
    "video_id",
    "timeout",
    "custom_llm_provider",
    "variant",
    "extra_headers",
    "extra_query",
    "extra_body",
];

pub(crate) fn content_request(wire: &VideoCallWire) -> Result<VideoContentRequest, ConvertError> {
    let arguments = &wire.arguments;
    Ok(VideoContentRequest {
        video_id: required_text(arguments, "video_id")?,
        timeout: duration(number(arguments, "timeout")?)?,
        custom_llm_provider: text(arguments, "custom_llm_provider")?,
        variant: text(arguments, "variant")?,
        extra_headers: text_map(arguments, "extra_headers")?,
        extra_query: params(arguments, "extra_query")?,
        extra_body: params(arguments, "extra_body")?,
        kwargs: loose(arguments, &CONTENT_NAMED),
    })
}

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

pub(crate) fn remix_request(wire: &VideoCallWire) -> Result<VideoRemixCall, ConvertError> {
    let arguments = &wire.arguments;
    Ok(VideoRemixCall {
        video_id: required_text(arguments, "video_id")?,
        prompt: required_text(arguments, "prompt")?,
        custom_llm_provider: text(arguments, "custom_llm_provider")?,
        api_key: secret(arguments, "api_key")?,
        api_base: text(arguments, "api_base")?,
        api_version: text(arguments, "api_version")?,
        extra_headers: text_map(arguments, "extra_headers")?,
        extra_query: params(arguments, "extra_query")?,
        extra_body: params(arguments, "extra_body")?,
        mock_response: arguments.get("mock_response").cloned(),
        litellm_metadata: metadata(arguments, "litellm_metadata")?,
        litellm_call_id: text(arguments, "litellm_call_id")?,
        user: text(arguments, "user")?,
        timeout: number(arguments, "timeout")?,
        extra: loose(arguments, &REMIX_NAMED),
    })
}

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

pub(crate) fn list_request(wire: &VideoCallWire) -> Result<VideoListCall, ConvertError> {
    let arguments = &wire.arguments;
    Ok(VideoListCall {
        after: text(arguments, "after")?,
        limit: integer(arguments, "limit")?,
        order: text(arguments, "order")?,
        custom_llm_provider: text(arguments, "custom_llm_provider")?,
        api_key: secret(arguments, "api_key")?,
        api_base: text(arguments, "api_base")?,
        api_version: text(arguments, "api_version")?,
        extra_headers: text_map(arguments, "extra_headers")?,
        extra_query: params(arguments, "extra_query")?,
        extra_body: params(arguments, "extra_body")?,
        mock_response: arguments.get("mock_response").cloned(),
        litellm_metadata: metadata(arguments, "litellm_metadata")?,
        litellm_call_id: text(arguments, "litellm_call_id")?,
        user: text(arguments, "user")?,
        timeout: number(arguments, "timeout")?,
        extra: loose(arguments, &LIST_NAMED),
    })
}

const STATUS_NAMED: [&str; 6] = [
    "video_id",
    "timeout",
    "custom_llm_provider",
    "extra_headers",
    "extra_query",
    "extra_body",
];

pub(crate) fn status_request(wire: &VideoCallWire) -> Result<VideoStatusRequest, ConvertError> {
    let arguments = &wire.arguments;
    Ok(VideoStatusRequest {
        video_id: required_text(arguments, "video_id")?,
        // `timeout or REQUEST_TIMEOUT`: an explicit null takes the request timeout, not the 600 default
        timeout: Some(duration(number(arguments, "timeout")?)?.unwrap_or(Duration::ZERO)),
        custom_llm_provider: text(arguments, "custom_llm_provider")?,
        extra_headers: text_map(arguments, "extra_headers")?,
        extra_query: params(arguments, "extra_query")?,
        extra_body: params(arguments, "extra_body")?,
        kwargs: loose(arguments, &STATUS_NAMED),
    })
}
