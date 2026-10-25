use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    pub safe_memory_mode: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub safe_memory_mode: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let safe_memory_mode = self.litellm_settings.safe_memory_mode.unwrap_or_else(|| {
            lookup
                .lookup("LITELLM_SAFE_MEMORY_MODE")
                .is_some_and(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
        });
        Ok(Settings { safe_memory_mode })
    }
}

#[path = "request_records/tests.rs"]
mod request_records_tests;
