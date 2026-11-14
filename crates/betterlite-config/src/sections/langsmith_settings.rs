use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    langsmith_batch_size: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    // None leaves the batch size to the logger's own default, which LiteLLM reads from DEFAULT_BATCH_SIZE
    pub batch_size: Option<i64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            batch_size: self.litellm_settings.langsmith_batch_size,
        })
    }
}

mod tests;
