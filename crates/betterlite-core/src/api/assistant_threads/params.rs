use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;

use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};

// LiteLLM's default when neither timeout nor request_timeout names one, for every provider here
pub(super) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
// both thread providers keep an httpx.Timeout instead of reading its read phase
const OPENAI: &str = "openai";

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(super) enum ThreadTimeoutValue {
    Seconds(f64),
    Text(String),
    Phases(StructuredTimeout),
}

impl ThreadTimeoutValue {
    // Python's `x or default`: a zero number, an empty string and an empty phase map are absent, and any other value is used
    const fn is_absent(&self) -> bool {
        match self {
            Self::Seconds(seconds) => *seconds == 0.0,
            Self::Text(text) => text.is_empty(),
            Self::Phases(phases) => {
                phases.connect.is_none()
                    && phases.read.is_none()
                    && phases.write.is_none()
                    && phases.pool.is_none()
            }
        }
    }

    // the kwargs value B2487's resolver reads, for the two shapes that are not structured
    fn as_raw(&self) -> Option<TimeoutValue> {
        match self {
            Self::Seconds(seconds) => Some(TimeoutValue::Number(*seconds)),
            Self::Text(text) => Some(TimeoutValue::String(text.clone())),
            Self::Phases(_) => None,
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
    // absent, null and an object are three different azure_ad_token sources, so the outer option stays
    #[serde(with = "serde_with::rust::double_option")]
    pub extra_body: Option<Option<BTreeMap<String, Json>>>,
}

impl ThreadParams {
    // GenericLiteLLMParams(**kwargs) ignores every key it doesn't declare
    pub(super) fn from_kwargs(kwargs: &BTreeMap<String, Json>) -> Result<Self, ThreadParamsError> {
        let bytes = serde_json::to_vec(kwargs).map_err(|source| ThreadParamsError {
            field: None,
            message: source.to_string(),
        })?;
        let mut decoder = serde_json::Deserializer::from_slice(&bytes);
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| ThreadParamsError {
            field: Some(error.path().to_string()),
            message: error.into_inner().to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ThreadParamsError {
    pub field: Option<String>,
    pub message: String,
}

fn seconds_of(timeout: FineTuningTimeout) -> ThreadTimeout {
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
    let raw = selected.and_then(ThreadTimeoutValue::as_raw).or_else(|| {
        params
            .request_timeout
            .filter(|seconds| *seconds != 0.0)
            .map(TimeoutValue::Number)
    });
    resolve_fine_tuning_timeout(raw.as_ref(), OPENAI, None).map(seconds_of)
}

// the raw kwargs timeout the Azure SDK client receives; the phases cannot travel, so its read phase stands in
pub(super) fn forwarded_azure_timeout(params: &ThreadParams) -> Result<Option<f64>, TimeoutError> {
    match params.timeout.as_ref() {
        None => Ok(None),
        Some(ThreadTimeoutValue::Phases(phases)) => {
            Ok(phases.read.filter(|seconds| *seconds > 0.0))
        }
        Some(ThreadTimeoutValue::Seconds(seconds)) if !seconds.is_finite() => {
            Err(TimeoutError::NonFinite)
        }
        Some(ThreadTimeoutValue::Seconds(seconds)) => Ok(Some(*seconds)),
        Some(ThreadTimeoutValue::Text(text)) => parse_text(text).map(Some),
    }
}

// Python's float() takes underscores between digits; httpx converts the same text the same way
fn parse_text(text: &str) -> Result<f64, TimeoutError> {
    let trimmed = text.trim();
    let valid_underscores = trimmed
        .chars()
        .zip(trimmed.chars().skip(1))
        .all(|(left, right)| {
            (left != '_' || right.is_ascii_digit()) && (right != '_' || left.is_ascii_digit())
        });
    let candidate = if valid_underscores && !trimmed.starts_with('_') && !trimmed.ends_with('_') {
        trimmed.replace('_', "")
    } else {
        trimmed.to_owned()
    };
    let seconds = candidate
        .parse::<f64>()
        .map_err(|source| TimeoutError::InvalidNumber { source })?;
    if seconds.is_finite() {
        Ok(seconds)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

// absent extra_body pops nothing, an explicit null reads AZURE_AD_TOKEN, and the body's token is popped and wins
pub(super) fn facade_azure_token(
    resources: &ResourceSdkSettings,
    params: &ThreadParams,
) -> Option<SecretString> {
    match &params.extra_body {
        None => None,
        Some(None) => resources.azure_ad_token.clone(),
        Some(Some(body)) => match body.get("azure_ad_token") {
            Some(Json::Text(token)) => Some(SecretString::from(token.clone())),
            Some(_) | None => None,
        },
    }
}
