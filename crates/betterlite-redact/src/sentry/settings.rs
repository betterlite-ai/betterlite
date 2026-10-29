use std::collections::BTreeMap;
use std::num::{ParseFloatError, ParseIntError};
use std::str::FromStr;

use super::fixture;

const SEND_DEFAULT_PII_ENV: &str = "SENTRY_SEND_DEFAULT_PII";
const MINIMUM_CUSTOM_KEY_LENGTH_ENV: &str = "MINIMUM_CUSTOM_KEY_LENGTH";
const GENERATED_KEY_LENGTH_ENV: &str = "LENGTH_OF_LITELLM_GENERATED_KEY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventScrubberConfig {
    pub denylist: Vec<String>,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SentryInitOptions {
    pub dsn: Option<String>,
    pub traces_sample_rate: f64,
    pub sample_rate: f64,
    pub send_default_pii: bool,
    pub environment: String,
    pub event_scrubber: EventScrubberConfig,
    pub custom_key_minimum: usize,
    pub generated_key_bytes: usize,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SettingsError {
    #[error("SENTRY_API_TRACE_RATE is not a valid float")]
    InvalidTraceRate {
        #[source]
        source: ParseFloatError,
    },
    #[error("SENTRY_API_SAMPLE_RATE is not a valid float")]
    InvalidSampleRate {
        #[source]
        source: ParseFloatError,
    },
    #[error("MINIMUM_CUSTOM_KEY_LENGTH is not a valid integer")]
    InvalidCustomKeyMinimum {
        #[source]
        source: ParseIntError,
    },
    #[error("LENGTH_OF_LITELLM_GENERATED_KEY is not a valid integer")]
    InvalidGeneratedKeyLength {
        #[source]
        source: ParseIntError,
    },
}

#[must_use]
pub fn send_default_pii_from_env(env: &BTreeMap<String, String>) -> bool {
    let raw = env.get(SEND_DEFAULT_PII_ENV);
    raw.is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

fn parse_env_value<T, E>(
    env: &BTreeMap<String, String>,
    key: &str,
    default: T,
    build_error: fn(E) -> SettingsError,
) -> Result<T, SettingsError>
where
    T: FromStr<Err = E>,
{
    match env.get(key) {
        None => Ok(default),
        Some(raw) if raw.is_empty() => Ok(default),
        Some(raw) => raw.trim().parse::<T>().map_err(build_error),
    }
}

fn event_scrubber_denylist(send_default_pii: bool) -> Vec<String> {
    let secret_names = fixture::DEFAULT_DENYLIST
        .iter()
        .chain(fixture::SENTRY_DENYLIST);
    let pii_names = fixture::DEFAULT_PII_DENYLIST
        .iter()
        .chain(fixture::SENTRY_PII_DENYLIST);
    if send_default_pii {
        secret_names.map(|name| name.to_lowercase()).collect()
    } else {
        secret_names
            .chain(pii_names)
            .map(|name| name.to_lowercase())
            .collect()
    }
}

pub fn build_sentry_init_options(
    env: &BTreeMap<String, String>,
) -> Result<SentryInitOptions, SettingsError> {
    let send_default_pii = send_default_pii_from_env(env);
    Ok(SentryInitOptions {
        dsn: env.get("SENTRY_DSN").cloned(),
        traces_sample_rate: parse_env_value(env, "SENTRY_API_TRACE_RATE", 1.0, |source| {
            SettingsError::InvalidTraceRate { source }
        })?,
        sample_rate: parse_env_value(env, "SENTRY_API_SAMPLE_RATE", 1.0, |source| {
            SettingsError::InvalidSampleRate { source }
        })?,
        send_default_pii,
        environment: env
            .get("SENTRY_ENVIRONMENT")
            .cloned()
            .unwrap_or_else(|| "production".to_owned()),
        event_scrubber: EventScrubberConfig {
            denylist: event_scrubber_denylist(send_default_pii),
            recursive: true,
        },
        custom_key_minimum: parse_env_value(
            env,
            MINIMUM_CUSTOM_KEY_LENGTH_ENV,
            fixture::MINIMUM_CUSTOM_KEY_LENGTH,
            |source| SettingsError::InvalidCustomKeyMinimum { source },
        )?,
        generated_key_bytes: parse_env_value(
            env,
            GENERATED_KEY_LENGTH_ENV,
            fixture::GENERATED_KEY_LENGTH,
            |source| SettingsError::InvalidGeneratedKeyLength { source },
        )?,
    })
}

mod tests;
