use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
    #[serde(default)]
    litellm_settings: RawLitellm,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    always_include_stream_usage: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellm {
    model_alias_map: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub always_include_stream_usage: Option<bool>,
    pub model_alias_map: BTreeMap<String, String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            always_include_stream_usage: self.general_settings.always_include_stream_usage,
            model_alias_map: self.litellm_settings.model_alias_map.unwrap_or_default(),
        })
    }
}
