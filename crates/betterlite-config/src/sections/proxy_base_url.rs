use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const PROXY_BASE_URL_NAME: &str = "PROXY_BASE_URL";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub proxy_base_url: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM tests PROXY_BASE_URL with `is not None`, so a set but empty value must stay Some("")
        Ok(Settings {
            proxy_base_url: lookup.lookup(PROXY_BASE_URL_NAME),
        })
    }
}
