use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::straiker::delivery::{
    DEFAULT_INITIAL_BACKOFF, DEFAULT_MAX_BACKOFF, DEFAULT_MAX_PAYLOAD_BYTES, DEFAULT_MAX_RETRIES,
    GUARDRAIL_NAME, WEBHOOK_PATH,
};
use crate::straiker::v3::{DETECT_PATH, KEY_PREFIX};
use crate::straiker_webhook::modes::{ModeError, configured_modes};
use crate::third_party_init_b::straiker::StraikerArgs;

const DEFAULT_TIMEOUT_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiVersion {
    V1,
    V3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatHint {
    AnthropicMessages,
    OpenAiChat,
}

impl FormatHint {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic.messages",
            Self::OpenAiChat => "openai.chat",
        }
    }
}

#[derive(Clone, Debug)]
pub struct StraikerSettings {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: ApiVersion,
    pub agent_ref: Option<String>,
    pub client: Option<String>,
    pub format_hint: Option<FormatHint>,
    pub source: String,
    pub timeout: Duration,
    pub max_retries: i64,
    pub initial_backoff: f64,
    pub max_backoff: f64,
    pub unreachable_fallback: UnreachableFallback,
    pub fail_on_error: bool,
    pub max_payload_bytes: usize,
    pub custom_headers: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, String>,
    pub verbose: bool,
    pub configured_modes: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("api_key must be non-empty")]
    EmptyApiKey,
    #[error("unreachable_fallback must be 'fail_open' or 'fail_closed'; got {value:?}")]
    UnreachableFallback { value: String },
    #[error("api_version must be 'v1' or 'v3'; got {value:?}")]
    ApiVersion { value: String },
    #[error("format_hint must be 'anthropic.messages' or 'openai.chat'; got {value:?}")]
    FormatHint { value: String },
    #[error("timeout must be a finite number of seconds; got {value}")]
    Timeout { value: f64 },
    #[error("initial_backoff must be a finite number of seconds; got {value}")]
    InitialBackoff { value: f64 },
    #[error("max_backoff must be a finite number of seconds; got {value}")]
    MaxBackoff { value: f64 },
    #[error("max_payload_bytes must be a non-negative integer; got {value}")]
    MaxPayloadBytes { value: i64 },
    #[error("event_hook could not be rendered as JSON: {0}")]
    EventHook(#[from] serde_json::Error),
    #[error(transparent)]
    Modes(#[from] ModeError),
}

impl StraikerSettings {
    pub fn resolve(raw: &StraikerArgs) -> Result<Self, SettingsError> {
        if raw.api_key.expose_secret().is_empty() {
            return Err(SettingsError::EmptyApiKey);
        }
        let unreachable_fallback = fallback_of(&raw.unreachable_fallback)?;
        let api_version = api_version_of(raw)?;
        let format_hint = format_hint_of(raw.format_hint.as_deref())?;
        let (initial_backoff, max_backoff) = backoffs(raw)?;
        Ok(Self {
            guardrail_name: name_of(raw),
            api_key: raw.api_key.clone(),
            api_base: raw.api_base.trim_end_matches('/').to_owned(),
            api_version,
            agent_ref: optional_text(raw.agent_ref.as_deref()),
            client: optional_text(raw.client.as_deref()),
            format_hint,
            source: raw.source.clone(),
            timeout: timeout_of(raw.timeout)?,
            max_retries: raw.max_retries.unwrap_or(DEFAULT_MAX_RETRIES).max(0),
            initial_backoff,
            max_backoff,
            unreachable_fallback,
            fail_on_error: raw.fail_on_error,
            max_payload_bytes: max_payload_bytes_of(raw.max_payload_bytes)?,
            custom_headers: raw.custom_headers.clone().unwrap_or_default(),
            metadata: raw.metadata.clone().unwrap_or_default(),
            verbose: raw.verbose.unwrap_or(false),
            configured_modes: configured_modes_of(&raw.event_hook)?,
        })
    }

    #[must_use]
    pub fn webhook_url(&self) -> String {
        let base = self.api_base.as_str();
        let path = match self.api_version {
            ApiVersion::V3 => DETECT_PATH,
            ApiVersion::V1 => WEBHOOK_PATH,
        };
        format!("{base}{path}")
    }
}

fn name_of(raw: &StraikerArgs) -> String {
    if raw.name.is_empty() {
        GUARDRAIL_NAME.to_owned()
    } else {
        raw.name.clone()
    }
}

fn optional_text(value: Option<&str>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        Some(_) | None => None,
    }
}

fn fallback_of(value: &str) -> Result<UnreachableFallback, SettingsError> {
    match value {
        "fail_closed" => Ok(UnreachableFallback::FailClosed),
        "fail_open" => Ok(UnreachableFallback::FailOpen),
        other => Err(SettingsError::UnreachableFallback {
            value: other.to_owned(),
        }),
    }
}

// An sk_agt_ key cannot drive the v1 webhook, so it routes to v3 whatever api_version says.
fn api_version_of(raw: &StraikerArgs) -> Result<ApiVersion, SettingsError> {
    let configured = match raw.api_version.as_deref() {
        None => None,
        Some("v1") => Some(ApiVersion::V1),
        Some("v3") => Some(ApiVersion::V3),
        Some(other) => {
            return Err(SettingsError::ApiVersion {
                value: other.to_owned(),
            });
        }
    };
    if raw.api_key.expose_secret().starts_with(KEY_PREFIX) {
        if configured == Some(ApiVersion::V1) {
            tracing::warn!(
                "Straiker guardrail: api_version 'v1' cannot use an sk_agt_ key, routing to /api/v3/detect"
            );
        }
        return Ok(ApiVersion::V3);
    }
    Ok(configured.unwrap_or(ApiVersion::V1))
}

fn format_hint_of(value: Option<&str>) -> Result<Option<FormatHint>, SettingsError> {
    match value {
        None => Ok(None),
        Some("anthropic.messages") => Ok(Some(FormatHint::AnthropicMessages)),
        Some("openai.chat") => Ok(Some(FormatHint::OpenAiChat)),
        Some(other) => Err(SettingsError::FormatHint {
            value: other.to_owned(),
        }),
    }
}

fn backoffs(raw: &StraikerArgs) -> Result<(f64, f64), SettingsError> {
    let initial = raw
        .initial_backoff
        .unwrap_or(DEFAULT_INITIAL_BACKOFF)
        .max(0.0_f64);
    if !initial.is_finite() {
        return Err(SettingsError::InitialBackoff { value: initial });
    }
    let max = raw.max_backoff.unwrap_or(DEFAULT_MAX_BACKOFF).max(initial);
    if !max.is_finite() {
        return Err(SettingsError::MaxBackoff { value: max });
    }
    Ok((initial, max))
}

fn timeout_of(seconds: Option<f64>) -> Result<Duration, SettingsError> {
    let Some(seconds) = seconds else {
        return Ok(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS));
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|_invalid| SettingsError::Timeout { value: seconds })
}

fn max_payload_bytes_of(bytes: Option<i64>) -> Result<usize, SettingsError> {
    let Some(bytes) = bytes else {
        return Ok(DEFAULT_MAX_PAYLOAD_BYTES);
    };
    usize::try_from(bytes).map_err(|_negative| SettingsError::MaxPayloadBytes { value: bytes })
}

fn configured_modes_of(
    event_hook: &LitellmParamsMode,
) -> Result<Option<Vec<String>>, SettingsError> {
    let encoded = serde_json::to_value(event_hook)?;
    let value: Json = serde_json::from_value(encoded)?;
    Ok(configured_modes(Some(&value))?)
}
