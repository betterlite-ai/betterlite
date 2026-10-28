use std::collections::BTreeMap;

use super::fixture;

const SEND_DEFAULT_PII_ENV: &str = "SENTRY_SEND_DEFAULT_PII";

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
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SettingsError {
    #[error("SENTRY_API_TRACE_RATE is not a valid float")]
    InvalidTraceRate,
    #[error("SENTRY_API_SAMPLE_RATE is not a valid float")]
    InvalidSampleRate,
}

#[must_use]
pub fn send_default_pii_from_env(env: &BTreeMap<String, String>) -> bool {
    let raw = env.get(SEND_DEFAULT_PII_ENV);
    raw.is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

fn parse_rate(
    env: &BTreeMap<String, String>,
    key: &str,
    on_error: SettingsError,
) -> Result<f64, SettingsError> {
    match env.get(key) {
        None => Ok(1.0),
        Some(raw) if raw.is_empty() => Ok(1.0),
        Some(raw) => raw.trim().parse::<f64>().map_err(|_parse_error| on_error),
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
        traces_sample_rate: parse_rate(
            env,
            "SENTRY_API_TRACE_RATE",
            SettingsError::InvalidTraceRate,
        )?,
        sample_rate: parse_rate(
            env,
            "SENTRY_API_SAMPLE_RATE",
            SettingsError::InvalidSampleRate,
        )?,
        send_default_pii,
        environment: env
            .get("SENTRY_ENVIRONMENT")
            .cloned()
            .unwrap_or_else(|| "production".to_owned()),
        event_scrubber: EventScrubberConfig {
            denylist: event_scrubber_denylist(send_default_pii),
            recursive: true,
        },
    })
}

mod tests;
