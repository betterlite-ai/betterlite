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
    #[serde(rename = "disable_vertex_batch_output_transformation")]
    disable_output_transformation: Option<bool>,
}

// the LiteLLM module global is off until a config sets it, and a null value leaves it off
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub disable_output_transformation: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            disable_output_transformation: self
                .litellm_settings
                .disable_output_transformation
                .unwrap_or(false),
        })
    }
}
