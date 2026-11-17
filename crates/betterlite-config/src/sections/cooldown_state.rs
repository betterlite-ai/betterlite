use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

const ENABLE_PATH: &str = "general_settings.enable_health_check_routing";
const STALENESS_PATH: &str = "general_settings.health_check_staleness_threshold";
const IGNORE_PATH: &str = "general_settings.health_check_ignore_transient_errors";
const INTERVAL_VARIABLE: &str = "DEFAULT_HEALTH_CHECK_INTERVAL";
const DEFAULT_HEALTH_CHECK_INTERVAL: f64 = 300.0;
const STALENESS_MULTIPLIER: f64 = 2.0;
const READ_INTERVAL_VARIABLE: &str = "DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS";
const DEFAULT_READ_INTERVAL_SECONDS: f64 = 1.0;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    enable_health_check_routing: Option<RawFlag>,
    #[serde(rename = "health_check_staleness_threshold")]
    staleness_threshold: Option<RawNumber>,
    #[serde(rename = "health_check_ignore_transient_errors")]
    ignore_transient_errors: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enable_health_check_routing: bool,
    pub staleness_threshold: f64,
    pub ignore_transient_errors: bool,
    pub read_interval_seconds: f64,
}

// the staleness default is LiteLLM's DEFAULT_HEALTH_CHECK_INTERVAL at import time, times its staleness multiplier
impl Default for Settings {
    fn default() -> Self {
        Self {
            enable_health_check_routing: false,
            staleness_threshold: DEFAULT_HEALTH_CHECK_INTERVAL.mul_add(STALENESS_MULTIPLIER, 0.0),
            ignore_transient_errors: false,
            read_interval_seconds: DEFAULT_READ_INTERVAL_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let enable = flag(general.enable_health_check_routing.as_ref(), ENABLE_PATH)?;
        let ignore = flag(general.ignore_transient_errors.as_ref(), IGNORE_PATH)?;
        let configured = resolve_number(
            general.staleness_threshold,
            STALENESS_PATH,
            lookup,
            parse_float,
        )?;
        // LiteLLM reads both environment settings at import, so a malformed one fails whatever the config says
        let interval = betterlite_env::number::<f64>(
            INTERVAL_VARIABLE,
            &lookup.lookup(INTERVAL_VARIABLE).unwrap_or_default(),
        )?
        .unwrap_or(DEFAULT_HEALTH_CHECK_INTERVAL);
        let read_interval = betterlite_env::number::<f64>(
            READ_INTERVAL_VARIABLE,
            &lookup.lookup(READ_INTERVAL_VARIABLE).unwrap_or_default(),
        )?
        .unwrap_or(DEFAULT_READ_INTERVAL_SECONDS);
        Ok(Settings {
            enable_health_check_routing: enable,
            staleness_threshold: configured
                .unwrap_or_else(|| interval.mul_add(STALENESS_MULTIPLIER, 0.0)),
            ignore_transient_errors: ignore,
            read_interval_seconds: read_interval,
        })
    }
}

fn flag(raw: Option<&RawFlag>, path: &str) -> Result<bool, ConfigError> {
    match raw {
        Some(RawFlag::Boolean(flag)) => Ok(*flag),
        Some(RawFlag::Other(_)) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
        None => Ok(false),
    }
}
