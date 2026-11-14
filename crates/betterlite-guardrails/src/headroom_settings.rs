use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

mod tests;

pub const BYPASS_HEADER: &str = "x-headroom-bypass";
pub const HEADROOM_RETRIEVE_TOOL_NAME: &str = "headroom_retrieve";
pub const HEADROOM_GUARDRAIL_PROVIDER: &str = "headroom";
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];
pub const DEFAULT_COMPRESS_TIMEOUT: Duration = Duration::from_secs(60);
pub const CONNECT_TIMEOUT_CAP: Duration = Duration::from_secs(5);

const API_BASE_ENV: &str = "HEADROOM_API_BASE";
const API_KEY_ENV: &str = "HEADROOM_API_KEY";
const FAIL_OPEN: &str = "fail_open";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressTimeout {
    pub total: Duration,
    pub connect: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct HeadroomInput {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub unreachable_fallback: Option<String>,
    pub timeout: Option<f64>,
    pub ccr_retrieval: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct HeadroomSettings {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: CompressTimeout,
    pub ccr_retrieval: bool,
}

#[derive(Debug, Error)]
pub enum HeadroomSettingsError {
    #[error(
        "Headroom guardrail requires an API base URL. Set `api_base` in the guardrail config or HEADROOM_API_BASE env var."
    )]
    MissingApiBase,
    #[error("the Headroom API key cannot be sent as a header value")]
    InvalidApiKey {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(
        "headroom config `unreachable_fallback` must be `fail_closed` or `fail_open`, got `{value}`"
    )]
    UnreachableFallback { value: String },
}

#[derive(Clone, Deserialize)]
#[serde(try_from = "RawHeadroomConfig")]
pub struct HeadroomConfig {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub ccr_retrieval: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeadroomConfig {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    unreachable_fallback: Option<String>,
    #[serde(default)]
    ccr_retrieval: Option<bool>,
}

impl TryFrom<RawHeadroomConfig> for HeadroomConfig {
    type Error = HeadroomSettingsError;

    fn try_from(raw: RawHeadroomConfig) -> Result<Self, Self::Error> {
        let unreachable_fallback = match raw.unreachable_fallback.as_deref() {
            None | Some("fail_closed") => UnreachableFallback::FailClosed,
            Some(FAIL_OPEN) => UnreachableFallback::FailOpen,
            Some(other) => {
                return Err(HeadroomSettingsError::UnreachableFallback {
                    value: other.to_owned(),
                });
            }
        };
        Ok(Self {
            api_base: raw.api_base,
            api_key: raw.api_key,
            model: raw.model,
            unreachable_fallback,
            ccr_retrieval: raw.ccr_retrieval.unwrap_or(true),
        })
    }
}

impl GuardrailConfigModelIdentity for HeadroomConfig {
    fn ui_friendly_name() -> &'static str {
        "Headroom"
    }
}

#[must_use]
pub fn resolve_timeout(timeout: Option<f64>) -> CompressTimeout {
    let total = timeout.map_or(DEFAULT_COMPRESS_TIMEOUT, |seconds| {
        usable_timeout(seconds).unwrap_or_else(|| {
            tracing::warn!("Headroom: ignoring unusable timeout {seconds}, using 60 seconds");
            DEFAULT_COMPRESS_TIMEOUT
        })
    });
    CompressTimeout {
        total,
        connect: total.min(CONNECT_TIMEOUT_CAP),
    }
}

fn usable_timeout(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

impl HeadroomSettings {
    pub fn new<L>(input: &HeadroomInput, secrets: &L) -> Result<Self, HeadroomSettingsError>
    where
        L: EnvironmentLookup,
    {
        let configured_base = input.api_base.as_deref().filter(|base| !base.is_empty());
        let secret_base = secrets.lookup(API_BASE_ENV);
        let api_base = configured_base
            .or_else(|| secret_base.as_deref().filter(|base| !base.is_empty()))
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_owned();
        if api_base.is_empty() {
            return Err(HeadroomSettingsError::MissingApiBase);
        }
        let api_key = input
            .api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| {
                secrets
                    .lookup(API_KEY_ENV)
                    .filter(|key| !key.is_empty())
                    .map(SecretString::from)
            });
        Ok(Self {
            api_base,
            api_key,
            model: input.model.clone(),
            unreachable_fallback: if input.unreachable_fallback.as_deref() == Some(FAIL_OPEN) {
                UnreachableFallback::FailOpen
            } else {
                UnreachableFallback::FailClosed
            },
            timeout: resolve_timeout(input.timeout),
            ccr_retrieval: input.ccr_retrieval.unwrap_or(true),
        })
    }

    pub fn request_headers(&self) -> Result<HeaderMap, HeadroomSettingsError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(key) = &self.api_key {
            let mut authorization =
                HeaderValue::from_str(&format!("Bearer {}", key.expose_secret()))
                    .map_err(|source| HeadroomSettingsError::InvalidApiKey { source })?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        Ok(headers)
    }
}
