#![expect(
    clippy::struct_excessive_bools,
    reason = "the five settings mirror LiteLLM's independent general_settings keys"
)]

mod tests;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const INTERVAL_VARIABLE: &str = "DEFAULT_HEALTH_CHECK_INTERVAL";
const DEFAULT_HEALTH_CHECK_INTERVAL: i64 = 300;
const BACKGROUND_PATH: &str = "general_settings.background_health_checks";
const SHARED_PATH: &str = "general_settings.use_shared_health_check";
const INTERVAL_PATH: &str = "general_settings.health_check_interval";
const CONCURRENCY_PATH: &str = "general_settings.health_check_concurrency";
const DETAILS_PATH: &str = "general_settings.health_check_details";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    background_health_checks: Option<RawFlag>,
    use_shared_health_check: Option<RawFlag>,
    health_check_interval: Option<RawInteger>,
    health_check_concurrency: Option<RawInteger>,
    health_check_details: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawInteger {
    Integer(i64),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub background_health_checks: bool,
    pub use_shared_health_check: bool,
    pub health_check_interval: i64,
    pub health_check_concurrency: Option<i64>,
    pub health_check_details: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_health_checks: false,
            use_shared_health_check: false,
            health_check_interval: DEFAULT_HEALTH_CHECK_INTERVAL,
            health_check_concurrency: None,
            health_check_details: true,
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
        let background_health_checks = flag(
            general.background_health_checks.as_ref(),
            false,
            BACKGROUND_PATH,
        )?;
        let use_shared_health_check =
            flag(general.use_shared_health_check.as_ref(), false, SHARED_PATH)?;
        let configured_interval = integer(general.health_check_interval.as_ref(), INTERVAL_PATH)?;
        // LiteLLM reads DEFAULT_HEALTH_CHECK_INTERVAL at import, so a malformed one fails whatever the config says
        let default_interval = betterlite_env::number::<i64>(
            INTERVAL_VARIABLE,
            &lookup.lookup(INTERVAL_VARIABLE).unwrap_or_default(),
        )?
        .unwrap_or(DEFAULT_HEALTH_CHECK_INTERVAL);
        Ok(Settings {
            background_health_checks,
            use_shared_health_check,
            health_check_interval: configured_interval.unwrap_or(default_interval),
            health_check_concurrency: integer(
                general.health_check_concurrency.as_ref(),
                CONCURRENCY_PATH,
            )?,
            health_check_details: flag(general.health_check_details.as_ref(), true, DETAILS_PATH)?,
        })
    }
}

fn flag(raw: Option<&RawFlag>, default: bool, path: &str) -> Result<bool, ConfigError> {
    match raw {
        None => Ok(default),
        Some(RawFlag::Boolean(value)) => Ok(*value),
        Some(RawFlag::Other(_)) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

fn integer(raw: Option<&RawInteger>, path: &str) -> Result<Option<i64>, ConfigError> {
    match raw {
        None => Ok(None),
        Some(RawInteger::Integer(value)) => Ok(Some(*value)),
        Some(RawInteger::Other(_)) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}
