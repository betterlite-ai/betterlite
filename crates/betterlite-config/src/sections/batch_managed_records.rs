use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const POLLING_VARIABLE: &str = "PROXY_BATCH_POLLING_ENABLED";

// the enterprise hook that registers the job never loads, so this flag is read and never turns polling on
#[derive(Debug, Default, Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub proxy_batch_polling_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            proxy_batch_polling_enabled: true,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = lookup.lookup(POLLING_VARIABLE).unwrap_or_default();
        let proxy_batch_polling_enabled =
            betterlite_env::flag(POLLING_VARIABLE, &text)?.unwrap_or(true);
        Ok(Settings {
            proxy_batch_polling_enabled,
        })
    }
}
