use std::collections::BTreeMap;

use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const POLLING_VARIABLE: &str = "PROXY_BATCH_POLLING_ENABLED";

// env-only section: no YAML key is read here, and the box keeps the map value type non-zero-sized for clippy
pub type Raw = BTreeMap<String, Box<IgnoredAny>>;

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
        // an unset variable reads as empty text, and the shared parser treats empty as unset
        let text = lookup.lookup(POLLING_VARIABLE).unwrap_or_default();
        let proxy_batch_polling_enabled =
            betterlite_env::flag(POLLING_VARIABLE, &text)?.unwrap_or(true);
        Ok(Settings {
            proxy_batch_polling_enabled,
        })
    }
}
