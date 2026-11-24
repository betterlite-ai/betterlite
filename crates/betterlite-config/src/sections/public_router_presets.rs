use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_AUTOROUTER_PRESETS_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/litellm/proxy/public_endpoints/autorouter_presets.json";
pub const AUTOROUTER_PRESETS_URL_ENV: &str = "LITELLM_AUTOROUTER_PRESETS_URL";
pub const LOCAL_AUTOROUTER_PRESETS_ENV: &str = "LITELLM_LOCAL_AUTOROUTER_PRESETS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    autorouter_presets_url: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Settings {
    pub local_only: bool,
    pub url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            local_only: false,
            url: DEFAULT_AUTOROUTER_PRESETS_URL.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM lowercases the flag without trimming it, so a padded value leaves remote fetch on
        let local_only = lookup
            .lookup(LOCAL_AUTOROUTER_PRESETS_ENV)
            .is_some_and(|value| value.to_lowercase() == "true");
        // the proxy assigns a configured litellm_settings key after import, so it beats the environment
        let url = self
            .litellm_settings
            .autorouter_presets_url
            .or_else(|| lookup.lookup(AUTOROUTER_PRESETS_URL_ENV))
            .unwrap_or_else(|| DEFAULT_AUTOROUTER_PRESETS_URL.to_owned());
        Ok(Settings { local_only, url })
    }
}

mod tests;
