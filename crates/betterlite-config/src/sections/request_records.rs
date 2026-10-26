use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    #[serde(default)]
    pub safe_memory_mode: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub safe_memory_mode: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            safe_memory_mode: self.litellm_settings.safe_memory_mode,
        })
    }
}

#[path = "request_records/tests.rs"]
mod request_records_tests;
