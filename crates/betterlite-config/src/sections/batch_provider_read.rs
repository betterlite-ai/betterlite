mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::resource_sdk_settings;

// the litellm_settings keys and their environment fallbacks are the ones resource_sdk_settings declares
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

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub resources: resource_sdk_settings::Settings,
    // retrieve_batch reads these three only through the anthropic branch
    pub anthropic_api_base: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub anthropic_api_key: Option<SecretString>,
}

impl Settings {
    // ANTHROPIC_API_BASE first, then ANTHROPIC_BASE_URL, skipping an empty value in either
    #[must_use]
    pub fn anthropic_base(&self) -> Option<&str> {
        [
            self.anthropic_api_base.as_deref(),
            self.anthropic_base_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
    }
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
        let resources = resource_sdk_settings::Raw {
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
        .convert(lookup)?;
        Ok(Settings {
            resources,
            // an empty value stays set here, as it does for the shared keys, and is skipped on use
            anthropic_api_base: lookup.lookup("ANTHROPIC_API_BASE"),
            anthropic_base_url: lookup.lookup("ANTHROPIC_BASE_URL"),
            anthropic_api_key: lookup.lookup("ANTHROPIC_API_KEY").map(SecretString::from),
        })
    }
}
