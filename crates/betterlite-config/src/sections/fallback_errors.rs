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
    #[serde(default)]
    expose_fallback_errors_to_caller: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct FallbackErrors {
    pub expose_to_caller: bool,
}

// the sections! macro names every section's settings type Settings
pub type Settings = FallbackErrors;

impl Section for Raw {
    type Settings = FallbackErrors;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(FallbackErrors {
            expose_to_caller: self
                .general_settings
                .expose_fallback_errors_to_caller
                .unwrap_or_default(),
        })
    }
}

mod tests;
