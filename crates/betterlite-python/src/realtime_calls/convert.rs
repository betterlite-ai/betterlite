use std::collections::BTreeMap;

use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_http_calls::error::RealtimeHttpCallsError;
use betterlite_models::decisions::Json;
use betterlite_provider::api::realtime_http_transport::error::RealtimeHttpError;
use indexmap::IndexMap;
use pyo3::PyErr;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use super::error::CallFailure;

mod tests;

#[derive(Debug, Error)]
pub(crate) enum ConvertError {
    #[error("the realtime call payload is not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the realtime call is missing its {0} argument")]
    Missing(&'static str),
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        CallFailure::class("ValueError", error.to_string()).exception()
    }
}

// main.py's payload: one "arguments" object, and unknown keys stay in extra the way LiteLLM's kwargs allow them
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Payload {
    pub(crate) arguments: Arguments,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Arguments {
    pub(crate) model: Option<String>,
    pub(crate) session: Option<Json>,
    pub(crate) expires_after: Option<Json>,
    pub(crate) transcription_session: Option<Json>,
    pub(crate) openai_ephemeral_key: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_version: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(flatten)]
    extra: IndexMap<String, Json>,
}

pub(crate) struct ClientSecretArguments {
    pub(crate) common: Common,
    pub(crate) session: Option<Json>,
    pub(crate) expires_after: Option<Json>,
}

pub(crate) struct TranscriptionArguments {
    pub(crate) common: Common,
    pub(crate) transcription_session: Option<Json>,
}

pub(crate) struct CallsArguments {
    pub(crate) common: Common,
    pub(crate) ephemeral_key: String,
    pub(crate) sdp: Vec<u8>,
    pub(crate) session: Option<Json>,
}

// The three HTTP calls' shared arguments; everything else in kwargs reaches only the pre-call log
pub(crate) struct Common {
    pub(crate) model: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_version: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

fn common(arguments: Arguments) -> Common {
    Common {
        model: arguments.model,
        timeout: arguments.timeout,
        api_base: arguments.api_base,
        api_key: arguments.api_key,
        api_version: arguments.api_version,
        extra_headers: arguments.extra_headers,
        kwargs: arguments.extra,
    }
}

pub(crate) fn client_secret(arguments: Arguments) -> ClientSecretArguments {
    ClientSecretArguments {
        session: arguments.session.clone(),
        expires_after: arguments.expires_after.clone(),
        common: common(arguments),
    }
}

pub(crate) fn transcription(arguments: Arguments) -> TranscriptionArguments {
    TranscriptionArguments {
        transcription_session: arguments.transcription_session.clone(),
        common: common(arguments),
    }
}

pub(crate) fn calls(
    arguments: Arguments,
    mut blobs: Vec<Vec<u8>>,
) -> Result<CallsArguments, ConvertError> {
    let Some(ephemeral_key) = arguments.openai_ephemeral_key.clone() else {
        return Err(ConvertError::Missing("openai_ephemeral_key"));
    };
    let Some(sdp) = blobs.pop() else {
        return Err(ConvertError::Missing("sdp_body"));
    };
    Ok(CallsArguments {
        session: arguments.session.clone(),
        common: common(arguments),
        ephemeral_key,
        sdp,
    })
}

// The provider's own exception keeps its class and fields; only the handler without a config lets the masked status escape
pub(crate) fn http_failure(error: &RealtimeHttpCallsError<RealtimeHttpError>) -> CallFailure {
    match error {
        RealtimeHttpCallsError::Dispatch(inner) => dispatch_failure(inner),
        RealtimeHttpCallsError::Resolution(source) => CallFailure {
            kind: String::from("BadRequestError"),
            message: source.to_string(),
            status_code: Some(400),
            headers: None,
        },
        RealtimeHttpCallsError::Record(source) => {
            CallFailure::class("ValueError", source.to_string())
        }
        RealtimeHttpCallsError::Logging(source) => {
            CallFailure::class("ValueError", source.to_string())
        }
        RealtimeHttpCallsError::Timeout { .. } => {
            CallFailure::class("ValueError", error.to_string())
        }
    }
}

fn dispatch_failure(error: &RealtimeHttpError) -> CallFailure {
    match error {
        RealtimeHttpError::Provider(source) => CallFailure {
            kind: String::from("BaseLLMException"),
            message: source.message.clone(),
            status_code: Some(source.status_code),
            headers: Some(text_headers(&source.headers)),
        },
        RealtimeHttpError::Status(source) => CallFailure {
            kind: String::from("MaskedHTTPStatusError"),
            message: source.message.clone(),
            status_code: Some(source.status_code),
            headers: Some(text_headers(&source.headers)),
        },
        RealtimeHttpError::Timeout(source) => CallFailure::class("Timeout", source.message.clone()),
        RealtimeHttpError::Transport(source) => {
            CallFailure::class("APIConnectionError", source.to_string())
        }
        RealtimeHttpError::Client(source) => {
            CallFailure::class("APIConnectionError", source.to_string())
        }
        RealtimeHttpError::Logging(source) => CallFailure::class("ValueError", source.to_string()),
        RealtimeHttpError::Sdp(source) => CallFailure::class("ValueError", source.to_string()),
        RealtimeHttpError::Config(source) => CallFailure::class("ValueError", source.to_string()),
        RealtimeHttpError::RequestUrl { .. }
        | RealtimeHttpError::HeaderName { .. }
        | RealtimeHttpError::HeaderValue { .. }
        | RealtimeHttpError::HeaderNotText { .. }
        | RealtimeHttpError::Encode(_) => CallFailure::class("ValueError", error.to_string()),
    }
}

fn text_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}
