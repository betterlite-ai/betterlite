use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const HSTS_NAME: &str = "LITELLM_ENABLE_HSTS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub enable_hsts: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM turns HSTS on only for "true", so "1", "yes" and "on" stay off
        let enable_hsts = lookup
            .lookup(HSTS_NAME)
            .is_some_and(|value| value.trim().to_lowercase() == "true");
        Ok(Settings { enable_hsts })
    }
}
