use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};

// LiteLLM's default when neither timeout nor request_timeout names one, for every provider here
pub(super) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
// both thread providers keep an httpx.Timeout instead of reading its read phase
const OPENAI: &str = "openai";

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout; serde_json buffers a float as a private number map that only serde_json::Number reads, so the value is taken as Json and converted
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "Json")]
pub(super) enum ThreadTimeoutValue {
    Seconds(f64),
    Text(String),
    Phases(StructuredTimeout),
}

#[derive(Debug, Error)]
#[error("timeout must be a number, a string or an object of phases")]
pub(super) struct InvalidTimeout;

impl TryFrom<Json> for ThreadTimeoutValue {
    type Error = InvalidTimeout;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        match value {
            Json::Number(number) => Ok(Self::Seconds(number.as_f64().ok_or(InvalidTimeout)?)),
            Json::Text(text) => Ok(Self::Text(text)),
            Json::Object(object) => Ok(Self::Phases(StructuredTimeout {
                connect: phase(object.get("connect"))?,
                read: phase(object.get("read"))?,
                write: phase(object.get("write"))?,
                pool: phase(object.get("pool"))?,
            })),
            Json::Null | Json::Bool(_) | Json::List(_) => Err(InvalidTimeout),
        }
    }
}

// httpx.Timeout takes each phase as a number; a null or missing phase keeps the client default
fn phase(value: Option<&Json>) -> Result<Option<f64>, InvalidTimeout> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number.as_f64().map(Some).ok_or(InvalidTimeout),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(InvalidTimeout)
        }
    }
}

impl ThreadTimeoutValue {
    // Python's `x or default`: a zero number, an empty string and an empty phase map are absent, and any other value is used
    fn is_absent(&self) -> bool {
        match self {
            Self::Seconds(seconds) => *seconds == 0.0,
            Self::Text(text) => text.is_empty(),
            Self::Phases(phases) => [phases.connect, phases.read, phases.write, phases.pool]
                .iter()
                .all(Option::is_none),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ThreadTimeout {
    Seconds(f64),
    Phases(StructuredTimeout),
}

impl ThreadTimeout {
    // the single deadline this transport can apply; a structured timeout's read phase stands for the whole call
    pub(super) fn deadline_seconds(self) -> f64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::Phases(phases) => phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        }
    }
}

// extra_body is read only for its azure_ad_token source, so absent, null and an object must stay apart
#[derive(Debug, Clone, Default)]
pub(super) enum ThreadExtraBody {
    #[default]
    Absent,
    Null,
    Body(BTreeMap<String, Json>),
}

// The kwargs create_thread and get_thread read: GenericLiteLLMParams' credential and retry fields plus extra_body.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct ThreadParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<ThreadTimeoutValue>,
    pub request_timeout: Option<f64>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
    #[serde(skip)]
    pub extra_body: ThreadExtraBody,
}

impl ThreadParams {
    // GenericLiteLLMParams(**kwargs) ignores every key it doesn't declare
    pub(super) fn from_kwargs(kwargs: &BTreeMap<String, Json>) -> Result<Self, ThreadParamsError> {
        let bytes = serde_json::to_vec(kwargs).map_err(|source| ThreadParamsError {
            field: None,
            message: source.to_string(),
            source: Some(source),
        })?;
        let mut decoder = serde_json::Deserializer::from_slice(&bytes);
        let mut params: Self = serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            let field = Some(error.path().to_string());
            let source = error.into_inner();
            ThreadParamsError {
                field,
                message: source.to_string(),
                source: Some(source),
            }
        })?;
        params.extra_body = match kwargs.get("extra_body") {
            None => ThreadExtraBody::Absent,
            Some(Json::Null) => ThreadExtraBody::Null,
            Some(Json::Object(body)) => ThreadExtraBody::Body(body.clone().into_iter().collect()),
            Some(other) => {
                return Err(ThreadParamsError {
                    field: Some(String::from("extra_body")),
                    message: format!("extra_body must be an object or null, found {other:?}"),
                    source: None,
                });
            }
        };
        Ok(params)
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ThreadParamsError {
    pub field: Option<String>,
    pub message: String,
    #[source]
    pub source: Option<serde_json::Error>,
}

const fn seconds_of(timeout: FineTuningTimeout) -> ThreadTimeout {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => ThreadTimeout::Seconds(seconds),
        // openai and azure keep the phases; this arm only guards the resolver's other providers
        FineTuningTimeout::Structured(phases) => ThreadTimeout::Phases(phases),
    }
}

// `timeout or request_timeout or 600`, then a structured timeout stays structured for openai and azure
pub(super) fn resolve_timeout(params: &ThreadParams) -> Result<ThreadTimeout, TimeoutError> {
    let selected = params.timeout.as_ref().filter(|value| !value.is_absent());
    if let Some(ThreadTimeoutValue::Phases(phases)) = selected {
        return resolve_fine_tuning_timeout(None, OPENAI, Some(*phases)).map(seconds_of);
    }
    let raw = match selected {
        Some(ThreadTimeoutValue::Seconds(seconds)) => Some(TimeoutValue::Number(*seconds)),
        Some(ThreadTimeoutValue::Text(text)) => Some(TimeoutValue::String(text.clone())),
        Some(ThreadTimeoutValue::Phases(_)) | None => params
            .request_timeout
            .filter(|seconds| *seconds != 0.0_f64)
            .map(TimeoutValue::Number),
    };
    resolve_fine_tuning_timeout(raw.as_ref(), OPENAI, None).map(seconds_of)
}

// the raw kwargs timeout the Azure SDK client receives; the phases cannot travel, so its read phase stands in
pub(super) fn forwarded_azure_timeout(params: &ThreadParams) -> Result<Option<f64>, TimeoutError> {
    let seconds = match params.timeout.as_ref() {
        None => return Ok(None),
        Some(ThreadTimeoutValue::Phases(phases)) => phases.read.unwrap_or(0.0_f64),
        Some(ThreadTimeoutValue::Seconds(seconds)) if !seconds.is_finite() => {
            return Err(TimeoutError::NonFinite);
        }
        Some(ThreadTimeoutValue::Seconds(seconds)) => *seconds,
        Some(ThreadTimeoutValue::Text(text)) if text.is_empty() => return Ok(None),
        Some(ThreadTimeoutValue::Text(text)) => {
            let raw = TimeoutValue::String(text.clone());
            seconds_of(resolve_fine_tuning_timeout(Some(&raw), OPENAI, None)?).deadline_seconds()
        }
    };
    // B176 refuses a negative timeout, so a non-positive one leaves its 600-second default
    Ok(Some(seconds).filter(|value| *value > 0.0_f64))
}

// absent extra_body pops nothing, an explicit null reads AZURE_AD_TOKEN, and the body's token is popped and wins
pub(super) fn facade_azure_token(
    resources: &ResourceSdkSettings,
    params: &ThreadParams,
) -> Option<SecretString> {
    match &params.extra_body {
        ThreadExtraBody::Absent => None,
        ThreadExtraBody::Null => resources.azure_ad_token.clone(),
        ThreadExtraBody::Body(body) => match body.get("azure_ad_token") {
            Some(Json::Text(token)) => Some(SecretString::from(token.clone())),
            Some(_) | None => None,
        },
    }
}
