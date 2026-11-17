use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    #[serde(default)]
    pub public_skills_index: Option<bool>,
}

// LiteLLM's litellm_settings.public_skills_index: the discovery routes answer 404 unless it is the YAML boolean true
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub public_skills_index: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            public_skills_index: self.settings.public_skills_index.unwrap_or(false),
        })
    }
}
