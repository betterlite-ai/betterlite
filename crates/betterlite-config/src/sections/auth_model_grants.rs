use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "enforce_fallback_model_access")]
    enforce_fallback_model_access: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enforce_fallback_model_access: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            enforce_fallback_model_access: self
                .general_settings
                .enforce_fallback_model_access
                .unwrap_or_default(),
        })
    }
}

mod tests;
