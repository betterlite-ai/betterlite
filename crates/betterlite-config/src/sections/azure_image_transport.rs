use std::time::Duration;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const DEFAULT_POLLING_TIMEOUT: Duration = Duration::from_secs(120);
const TIMEOUT_ENV: &str = "AZURE_OPERATION_POLLING_TIMEOUT";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub operation_polling_timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            operation_polling_timeout: DEFAULT_POLLING_TIMEOUT,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let seconds = betterlite_env::number::<i64>(
            TIMEOUT_ENV,
            &lookup.lookup(TIMEOUT_ENV).unwrap_or_default(),
        )
        .map_err(|source| ConfigError::Environment { source })?;
        Ok(Settings {
            operation_polling_timeout: seconds.map_or(DEFAULT_POLLING_TIMEOUT, |value| {
                Duration::from_secs(value.max(0).unsigned_abs())
            }),
        })
    }
}
