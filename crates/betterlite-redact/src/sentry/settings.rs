use std::collections::BTreeMap;

use super::fixture;

pub const SEND_DEFAULT_PII_ENV: &str = "SENTRY_SEND_DEFAULT_PII";
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
    #[error(transparent)]
    Environment {
        #[from]
        source: betterlite_env::EnvError,
    },
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
    let text = |name| env.get(name).map_or("", String::as_str);
    let send_default_pii =
        betterlite_env::flag(SEND_DEFAULT_PII_ENV, text(SEND_DEFAULT_PII_ENV))?.unwrap_or(false);
    Ok(SentryInitOptions {
        dsn: env.get("SENTRY_DSN").cloned(),
        traces_sample_rate: betterlite_env::number::<f64>(
            "SENTRY_API_TRACE_RATE",
            text("SENTRY_API_TRACE_RATE"),
        )?
        .unwrap_or(1.0),
        sample_rate: betterlite_env::number::<f64>(
            "SENTRY_API_SAMPLE_RATE",
            text("SENTRY_API_SAMPLE_RATE"),
        )?
        .unwrap_or(1.0),
        send_default_pii,
        environment: env
            .get("SENTRY_ENVIRONMENT")
            .cloned()
            .unwrap_or_else(|| "production".to_owned()),
        event_scrubber: EventScrubberConfig {
            denylist: event_scrubber_denylist(send_default_pii),
            recursive: true,
        },
        custom_key_minimum: betterlite_env::number::<usize>(
            MINIMUM_CUSTOM_KEY_LENGTH_ENV,
            text(MINIMUM_CUSTOM_KEY_LENGTH_ENV),
        )?
        .unwrap_or(fixture::MINIMUM_CUSTOM_KEY_LENGTH),
        generated_key_bytes: betterlite_env::number::<usize>(
            GENERATED_KEY_LENGTH_ENV,
            text(GENERATED_KEY_LENGTH_ENV),
        )?
        .unwrap_or(fixture::GENERATED_KEY_LENGTH),
    })
}

mod tests;
