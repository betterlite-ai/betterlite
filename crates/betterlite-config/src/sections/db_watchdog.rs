use std::time::Duration;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ENABLED_VARIABLE: &str = "PRISMA_HEALTH_WATCHDOG_ENABLED";
const INTERVAL_VARIABLE: &str = "PRISMA_HEALTH_WATCHDOG_INTERVAL_SECONDS";
const PROBE_TIMEOUT_VARIABLE: &str = "PRISMA_HEALTH_WATCHDOG_PROBE_TIMEOUT_SECONDS";
const RECONNECT_TIMEOUT_VARIABLE: &str = "PRISMA_WATCHDOG_RECONNECT_TIMEOUT_SECONDS";
const SCHEMA_VARIABLE: &str = "DATABASE_SCHEMA";

const DEFAULT_INTERVAL_SECS: i64 = 30;
const MIN_INTERVAL_SECS: i64 = 5;
const DEFAULT_PROBE_TIMEOUT_SECS: f64 = 5.0;
const MIN_PROBE_TIMEOUT_SECS: f64 = 0.5;
const DEFAULT_RECONNECT_TIMEOUT_SECS: f64 = 30.0;
const MIN_RECONNECT_TIMEOUT_SECS: f64 = 1.0;
const DEFAULT_SCHEMA: &str = "public";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // every value is read from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub interval: Duration,
    pub probe_timeout: Duration,
    pub reconnect_timeout: Duration,
    pub database_schema: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS.unsigned_abs()),
            probe_timeout: Duration::try_from_secs_f64(DEFAULT_PROBE_TIMEOUT_SECS)
                .unwrap_or(Duration::MAX),
            reconnect_timeout: Duration::try_from_secs_f64(DEFAULT_RECONNECT_TIMEOUT_SECS)
                .unwrap_or(Duration::MAX),
            database_schema: DEFAULT_SCHEMA.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            enabled: enabled_setting(lookup)?,
            interval: interval_setting(lookup)?,
            probe_timeout: timeout_setting(
                lookup,
                PROBE_TIMEOUT_VARIABLE,
                DEFAULT_PROBE_TIMEOUT_SECS,
                MIN_PROBE_TIMEOUT_SECS,
            )?,
            reconnect_timeout: timeout_setting(
                lookup,
                RECONNECT_TIMEOUT_VARIABLE,
                DEFAULT_RECONNECT_TIMEOUT_SECS,
                MIN_RECONNECT_TIMEOUT_SECS,
            )?,
            database_schema: schema_setting(lookup),
        })
    }
}

fn enabled_setting<L: EnvironmentLookup>(lookup: &L) -> Result<bool, ConfigError> {
    let Some(text) = lookup.lookup(ENABLED_VARIABLE) else {
        return Ok(true);
    };
    let flag = betterlite_env::flag(ENABLED_VARIABLE, &text)?;
    Ok(flag.unwrap_or(true))
}

fn interval_setting<L>(lookup: &L) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = lookup
        .lookup(INTERVAL_VARIABLE)
        .map(|text| betterlite_env::number::<i64>(INTERVAL_VARIABLE, &text))
        .transpose()?
        .flatten();
    let bounded = configured
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Ok(Duration::from_secs(bounded.unsigned_abs()))
}

fn timeout_setting<L>(
    lookup: &L,
    name: &str,
    default: f64,
    minimum: f64,
) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = match lookup.lookup(name) {
        Some(text) => betterlite_env::number::<f64>(name, &text)?,
        None => None,
    };
    // Python's max(minimum, value): NaN and negative infinity select the minimum, positive infinity stays
    let selected = configured.unwrap_or(default).max(minimum);
    Ok(Duration::try_from_secs_f64(selected).unwrap_or(Duration::MAX))
}

fn schema_setting<L: EnvironmentLookup>(lookup: &L) -> String {
    lookup
        .lookup(SCHEMA_VARIABLE)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SCHEMA.to_owned())
}
