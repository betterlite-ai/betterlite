use std::time::Duration;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// constants.DEFAULT_REDIS_SYNC_INTERVAL: int(os.getenv("DEFAULT_REDIS_SYNC_INTERVAL", 1))
pub const DEFAULT_SYNC_SECONDS: u64 = 1;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub default_sync_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_sync_interval: Duration::from_secs(DEFAULT_SYNC_SECONDS),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = lookup
            .lookup("DEFAULT_REDIS_SYNC_INTERVAL")
            .unwrap_or_default();
        let seconds = betterlite_env::number::<u64>("DEFAULT_REDIS_SYNC_INTERVAL", &text)?
            .unwrap_or(DEFAULT_SYNC_SECONDS);
        Ok(Settings {
            default_sync_interval: Duration::from_secs(seconds),
        })
    }
}
