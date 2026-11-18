use std::collections::BTreeMap;

use betterlite_env::EnvError;
use serde::de::IgnoredAny;
use tracing::warn;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const PROVIDER_VARIABLE: &str = "FOCUS_PROVIDER";
pub const FORMAT_VARIABLE: &str = "FOCUS_FORMAT";
pub const FREQUENCY_VARIABLE: &str = "FOCUS_FREQUENCY";
pub const CRON_OFFSET_VARIABLE: &str = "FOCUS_CRON_OFFSET";
pub const INTERVAL_SECONDS_VARIABLE: &str = "FOCUS_INTERVAL_SECONDS";
pub const PREFIX_VARIABLE: &str = "FOCUS_PREFIX";

pub const DEFAULT_PROVIDER: &str = "s3";
pub const DEFAULT_EXPORT_FORMAT: &str = "parquet";
pub const DEFAULT_FREQUENCY: &str = "hourly";
pub const DEFAULT_CRON_OFFSET_MINUTE: i64 = 5;
pub const DEFAULT_INTERVAL_SECONDS: i64 = 60;
pub const DEFAULT_PREFIX: &str = "focus_exports";

// env-only section: no YAML key is read here, and the newtype keeps this Section impl off openai_realtime's map type
#[derive(Default, serde::Deserialize)]
#[serde(transparent)]
pub struct Raw(pub BTreeMap<String, Box<IgnoredAny>>);

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inner {
    pub provider: String,
    pub export_format: String,
    pub frequency: String,
    pub cron_offset_minute: i64,
    pub interval_seconds: Option<i64>,
    pub prefix: String,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            provider: DEFAULT_PROVIDER.to_owned(),
            export_format: DEFAULT_EXPORT_FORMAT.to_owned(),
            frequency: DEFAULT_FREQUENCY.to_owned(),
            cron_offset_minute: DEFAULT_CRON_OFFSET_MINUTE,
            interval_seconds: None,
            prefix: DEFAULT_PREFIX.to_owned(),
        }
    }
}

impl Settings {
    // the source's __init__ defaults: int(FOCUS_CRON_OFFSET) has no fallback, an invalid FOCUS_INTERVAL_SECONDS warns
    pub fn from_environment<L>(lookup: &L) -> Result<Self, EnvError>
    where
        L: EnvironmentLookup + ?Sized,
    {
        let cron_offset = lookup.lookup(CRON_OFFSET_VARIABLE).unwrap_or_default();
        let cron_offset_minute = betterlite_env::number::<i64>(CRON_OFFSET_VARIABLE, &cron_offset)?
            .unwrap_or(DEFAULT_CRON_OFFSET_MINUTE);
        let interval_seconds = lookup.lookup(INTERVAL_SECONDS_VARIABLE).and_then(|raw| {
            betterlite_env::number::<i64>(INTERVAL_SECONDS_VARIABLE, &raw).unwrap_or_else(
                |_error| {
                    warn!("Invalid FOCUS_INTERVAL_SECONDS value: {raw}, ignoring");
                    None
                },
            )
        });
        Ok(Self {
            inner: Box::new(Inner {
                provider: lowercased(lookup, PROVIDER_VARIABLE, DEFAULT_PROVIDER),
                export_format: lowercased(lookup, FORMAT_VARIABLE, DEFAULT_EXPORT_FORMAT),
                frequency: lowercased(lookup, FREQUENCY_VARIABLE, DEFAULT_FREQUENCY),
                cron_offset_minute,
                interval_seconds,
                prefix: match lookup.lookup(PREFIX_VARIABLE) {
                    Some(value) if !value.is_empty() => value,
                    _ => DEFAULT_PREFIX.to_owned(),
                },
            }),
        })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings::from_environment(lookup)?)
    }
}

// os.getenv(NAME) or the default, lowercased: an empty value counts as unset
fn lowercased<L>(lookup: &L, variable: &str, default: &str) -> String
where
    L: EnvironmentLookup + ?Sized,
{
    match lookup.lookup(variable) {
        Some(value) if !value.is_empty() => value.to_lowercase(),
        _ => default.to_owned(),
    }
}
