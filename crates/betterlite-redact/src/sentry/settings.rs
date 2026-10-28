//! Pure settings assembly for `build_sentry_init_options`: environment parsing and
//! the event-scrubber configuration, without an actual Sentry SDK call.

use std::collections::BTreeMap;

use super::fixture;

const SEND_DEFAULT_PII_ENV: &str = "SENTRY_SEND_DEFAULT_PII";

/// Mirrors the two attributes `EventScrubber.__init__` keeps on the instance.
///
/// It folds `pii_denylist` into `denylist` (when PII sending is off) and
/// lowercases the merged list before storing it.
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

fn str_to_bool(value: Option<&String>) -> Option<bool> {
    match value?.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Mirrors `send_default_pii_from_env`: only the literal string `"true"` (after
/// trimming and lowercasing) turns PII sending on; everything else, including a
/// missing variable, keeps it off.
#[must_use]
pub fn send_default_pii_from_env(env: &BTreeMap<String, String>) -> bool {
    str_to_bool(env.get(SEND_DEFAULT_PII_ENV)) == Some(true)
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
    let mut names = fixture::secret_field_names();
    if !send_default_pii {
        names.extend(fixture::pii_field_names());
    }
    names.into_iter().map(str::to_lowercase).collect()
}

/// Mirrors `build_sentry_init_options`: assembles pure settings, with no Sentry SDK
/// call of its own.
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
