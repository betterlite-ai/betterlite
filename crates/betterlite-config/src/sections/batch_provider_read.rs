mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::resource_sdk_settings;

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
        match (
            self.anthropic_api_base.as_deref(),
            self.anthropic_base_url.as_deref(),
        ) {
            (Some(api_base), _) if !api_base.is_empty() => Some(api_base),
            (_, Some(base_url)) if !base_url.is_empty() => Some(base_url),
            (_, _) => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: resource_sdk_settings::RawSettings,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let resources = resource_sdk_settings::Raw {
            settings: self.litellm_settings,
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
