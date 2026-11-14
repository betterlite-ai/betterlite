mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::resource_sdk_settings;

// the keys and their environment fallbacks are the ones resource_sdk_settings already declares
pub type Settings = resource_sdk_settings::Settings;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: RawSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.litellm_settings;
        resource_sdk_settings::Raw {
            settings: resource_sdk_settings::RawSettings {
                api_base: settings.api_base,
                api_version: settings.api_version,
                api_key: settings.api_key,
                openai_key: settings.openai_key,
                azure_key: settings.azure_key,
                organization: settings.organization,
                vertex_project: settings.vertex_project,
                vertex_location: settings.vertex_location,
            },
        }
        .convert(lookup)
    }
}
