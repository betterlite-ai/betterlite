use std::time::Duration;

use betterlite_env::EnvError;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const DRAIN_TOKEN_PATH: &str = "general_settings.drain_endpoint_token";
const REQUEST_TIMEOUT_PATH: &str = "litellm_settings.request_timeout";
const GRACEFUL_TIMEOUT_VARIABLE: &str = "GRACEFUL_SHUTDOWN_TIMEOUT";
const DRAIN_TOKEN_VARIABLE: &str = "DRAIN_ENDPOINT_TOKEN";
const STALL_WINDOW_VARIABLE: &str = "PROXY_DB_LOOKUP_STALL_WINDOW_SECONDS";
const NO_REDIS_VARIABLE: &str = "LITELLM_DISABLE_NO_REDIS_WARNING";
const REQUEST_TIMEOUT_VARIABLE: &str = "REQUEST_TIMEOUT";
const DEFAULT_GRACEFUL_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_STALL_WINDOW_SECONDS: f64 = 30.0;
const DEFAULT_REQUEST_TIMEOUT: f64 = 6000.0;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "allow_public_health_readiness_details")]
    allow_public_details: Option<RawFlag>,
    #[serde(rename = "enable_drain_endpoint")]
    enable_drain: Option<RawFlag>,
    #[serde(rename = "drain_endpoint_token")]
    drain_token: Option<RawToken>,
    alerting: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    request_timeout: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawToken {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default)]
pub struct DrainSettings {
    pub enabled: bool,
    pub token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub allow_public_details: bool,
    pub drain: DrainSettings,
    pub graceful_shutdown_timeout: Duration,
    pub db_lookup_stall_window_seconds: f64,
    pub disable_no_redis_warning: bool,
    pub request_timeout: f64,
    pub alerting: Option<Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            allow_public_details: false,
            drain: DrainSettings::default(),
            graceful_shutdown_timeout: DEFAULT_GRACEFUL_TIMEOUT,
            db_lookup_stall_window_seconds: DEFAULT_STALL_WINDOW_SECONDS,
            disable_no_redis_warning: false,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            alerting: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let no_redis_text = lookup.lookup(NO_REDIS_VARIABLE).unwrap_or_default();
        let disable_no_redis_warning =
            betterlite_env::flag(NO_REDIS_VARIABLE, &no_redis_text)?.unwrap_or(false);
        let general = self.general_settings;
        Ok(Settings {
            // LiteLLM reads both switches with `is True`, so a string "true" leaves them off
            allow_public_details: matches!(
                general.allow_public_details,
                Some(RawFlag::Boolean(true))
            ),
            drain: DrainSettings {
                enabled: matches!(general.enable_drain, Some(RawFlag::Boolean(true))),
                token: drain_token(general.drain_token, lookup)?,
            },
            graceful_shutdown_timeout: graceful_shutdown_timeout(lookup),
            db_lookup_stall_window_seconds: stall_window(lookup)?,
            disable_no_redis_warning,
            request_timeout: request_timeout(self.litellm_settings.request_timeout, lookup)?,
            alerting: general.alerting,
        })
    }
}

// the kubelet preStop hook can supply the token through the environment without a config reload
fn drain_token<L>(
    configured: Option<RawToken>,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = configured_token(configured, lookup)?;
    let environment = lookup
        .lookup(DRAIN_TOKEN_VARIABLE)
        .filter(|value| !value.is_empty());
    Ok(configured.or(environment).map(SecretString::from))
}

// LiteLLM resolves an unset os.environ reference to None, which then falls through to the env var
fn configured_token<L>(
    configured: Option<RawToken>,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(RawToken::Text(text)) = configured else {
        return Ok(None);
    };
    let unset = text
        .strip_prefix("os.environ/")
        .is_some_and(|variable| lookup.lookup(variable).is_none());
    if unset {
        return Ok(None);
    }
    let resolved = resolve_string(text, DRAIN_TOKEN_PATH, lookup)?;
    Ok(Some(resolved).filter(|value| !value.is_empty()))
}

// a malformed timeout must not keep the pod alive past terminationGracePeriodSeconds, so it falls back to 30 s
fn graceful_shutdown_timeout<L>(lookup: &L) -> Duration
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(GRACEFUL_TIMEOUT_VARIABLE) else {
        return DEFAULT_GRACEFUL_TIMEOUT;
    };
    // Python's float() reads inf and nan words, so betterlite_env::number's digit-only check does not fit
    let Ok(seconds) = text.trim().parse::<f64>() else {
        tracing::warn!(
            variable = GRACEFUL_TIMEOUT_VARIABLE,
            value = %text,
            "the graceful shutdown timeout is not a number; using 30 seconds"
        );
        return DEFAULT_GRACEFUL_TIMEOUT;
    };
    // Python compares a NaN deadline false on both sides, so the drain waits until requests finish
    if seconds.is_nan() {
        return Duration::MAX;
    }
    Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or(Duration::MAX)
}

fn stall_window<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = lookup.lookup(STALL_WINDOW_VARIABLE).unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(DEFAULT_STALL_WINDOW_SECONDS);
    }
    // Python's float() reads inf and nan words, so betterlite_env::number's digit-only check does not fit
    let seconds = text
        .trim()
        .parse::<f64>()
        .map_err(|_source| EnvError::Number {
            name: STALL_WINDOW_VARIABLE.to_owned(),
        })?;
    Ok(seconds.max(0.0))
}

// the same resolution B002's router timeouts apply: the config value, then REQUEST_TIMEOUT, then 6000
fn request_timeout<L>(configured: Option<RawNumber>, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = resolve_number(configured, REQUEST_TIMEOUT_PATH, lookup, parse_float)?;
    let environment_text = lookup.lookup(REQUEST_TIMEOUT_VARIABLE).unwrap_or_default();
    let environment = betterlite_env::number::<f64>(REQUEST_TIMEOUT_VARIABLE, &environment_text)?;
    Ok(configured
        .or(environment)
        .unwrap_or(DEFAULT_REQUEST_TIMEOUT))
}
