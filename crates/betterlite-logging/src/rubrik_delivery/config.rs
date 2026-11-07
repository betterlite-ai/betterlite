use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::error::RubrikError;
use super::transport::{LOGGING_TIMEOUT, MODERATION_TIMEOUT, Timeout};

pub const RESPONSE_MODERATION_PATH: &str = "/v1/after_completion/openai/v1";
pub const PROMPT_MODERATION_PATH: &str = "/v1/before_prompt/openai/v1";
pub const LOGGING_BATCH_PATH: &str = "/v1/litellm/batch";
pub const MAX_QUEUE_SIZE: usize = 10_000;
pub const DROP_WARNING_INTERVAL: Duration = Duration::from_secs(60);
pub const DEFAULT_BATCH_SIZE: usize = 512;
pub const DEFAULT_GUARDRAIL_NAME: &str = "rubrik";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubrikEventHook {
    PreCall,
    PostCall,
}

impl RubrikEventHook {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreCall => "pre_call",
            Self::PostCall => "post_call",
        }
    }
}

pub const SUPPORTED_EVENT_HOOKS: [RubrikEventHook; 2] =
    [RubrikEventHook::PreCall, RubrikEventHook::PostCall];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFlags {
    pub end_of_stream_only: bool,
    pub buffer_until_moderated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RubrikInit {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub event_hook: Option<RubrikEventHook>,
    pub default_on: Option<bool>,
    pub guardrail_name: Option<String>,
    pub timeout: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct RubrikSettings {
    pub guardrail_name: String,
    pub event_hook: RubrikEventHook,
    pub default_on: bool,
    pub api_key: Option<SecretString>,
    pub sampling_rate: f64,
    pub batch_size: usize,
    pub max_queue_size: usize,
    pub stream: StreamFlags,
    pub moderation_timeout: Timeout,
    pub logging_timeout: Timeout,
    pub response_moderation_endpoint: String,
    pub prompt_moderation_endpoint: String,
    pub logging_endpoint: String,
}

impl RubrikSettings {
    pub fn resolve(
        init: &RubrikInit,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, RubrikError> {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let sampling_rate = parse_sampling_rate(environment);
        let api_key = init
            .api_key
            .clone()
            .or_else(|| read("RUBRIK_API_KEY").map(SecretString::from));
        if api_key.is_none() {
            tracing::warn!("Rubrik: No API key configured. Requests will be unauthenticated.");
        }
        let batch_size = parse_batch_size(environment);
        let base = webhook_base(init, environment)?;
        let moderation_timeout = init.timeout.map_or(MODERATION_TIMEOUT, |timeout| Timeout {
            total: timeout,
            connect: timeout,
        });
        Ok(Self {
            guardrail_name: init
                .guardrail_name
                .clone()
                .unwrap_or_else(|| DEFAULT_GUARDRAIL_NAME.to_owned()),
            event_hook: init.event_hook.unwrap_or(RubrikEventHook::PostCall),
            default_on: init.default_on.unwrap_or(false),
            api_key,
            sampling_rate,
            batch_size,
            max_queue_size: MAX_QUEUE_SIZE,
            stream: StreamFlags {
                end_of_stream_only: true,
                buffer_until_moderated: true,
            },
            moderation_timeout,
            logging_timeout: LOGGING_TIMEOUT,
            response_moderation_endpoint: format!("{base}{RESPONSE_MODERATION_PATH}"),
            prompt_moderation_endpoint: format!("{base}{PROMPT_MODERATION_PATH}"),
            logging_endpoint: format!("{base}{LOGGING_BATCH_PATH}"),
        })
    }

    pub fn headers(&self) -> Result<HeaderMap, RubrikError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let Some(api_key) = &self.api_key else {
            return Ok(headers);
        };
        let bearer = format!("Bearer {}", api_key.expose_secret());
        let mut value = HeaderValue::from_str(&bearer)
            .map_err(|source| RubrikError::InvalidApiKey { source })?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
        Ok(headers)
    }
}

fn webhook_base(
    init: &RubrikInit,
    environment: &dyn EnvironmentLookup,
) -> Result<String, RubrikError> {
    let from_environment = environment
        .lookup("RUBRIK_WEBHOOK_URL")
        .filter(|value| !value.is_empty());
    let Some(webhook_url) = init.api_base.clone().or(from_environment) else {
        return Err(RubrikError::MissingWebhookUrl);
    };
    let without_slashes = webhook_url.trim_end_matches('/');
    Ok(without_slashes
        .strip_suffix("/v1")
        .unwrap_or(without_slashes)
        .to_owned())
}

fn parse_sampling_rate(environment: &dyn EnvironmentLookup) -> f64 {
    let Some(text) = environment.lookup("RUBRIK_SAMPLING_RATE") else {
        return 1.0;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 1.0;
    }
    let Ok(parsed) = trimmed.parse::<f64>() else {
        tracing::warn!(value = %text, "Invalid RUBRIK_SAMPLING_RATE, using 1.0");
        return 1.0;
    };
    // Python's max(0.0, min(1.0, nan)) is 1.0, while f64::clamp returns the NaN itself
    let clamped = if parsed.is_nan() {
        1.0_f64
    } else {
        parsed.clamp(0.0_f64, 1.0_f64)
    };
    if !(0.0_f64..=1.0_f64).contains(&parsed) {
        tracing::warn!(
            rate = parsed,
            clamped,
            "RUBRIK_SAMPLING_RATE clamped to [0, 1]"
        );
    }
    clamped
}

fn parse_batch_size(environment: &dyn EnvironmentLookup) -> usize {
    let Some(text) = environment.lookup("RUBRIK_BATCH_SIZE") else {
        return DEFAULT_BATCH_SIZE;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return DEFAULT_BATCH_SIZE;
    }
    let Ok(parsed) = trimmed.parse::<i64>() else {
        tracing::warn!(value = %text, "Invalid RUBRIK_BATCH_SIZE, using the default");
        return DEFAULT_BATCH_SIZE;
    };
    if parsed <= 0 {
        tracing::warn!(value = %text, "RUBRIK_BATCH_SIZE must be > 0, using the default");
        return DEFAULT_BATCH_SIZE;
    }
    usize::try_from(parsed).unwrap_or_else(|_| {
        tracing::warn!(value = %text, "RUBRIK_BATCH_SIZE is too large, using the default");
        DEFAULT_BATCH_SIZE
    })
}
