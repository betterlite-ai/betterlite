use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_BETA_HEADERS_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/litellm/anthropic_beta_headers_config.json";
pub const LOCAL_BETA_HEADERS_ENV: &str = "LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS";
pub const BETA_HEADERS_URL_ENV: &str = "LITELLM_ANTHROPIC_BETA_HEADERS_URL";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    anthropic_beta_headers_url: Option<String>,
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
            url: DEFAULT_BETA_HEADERS_URL.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM compares the lowercased value to "true" without trimming, and an empty URL stays empty
        let local_only = lookup
            .lookup(LOCAL_BETA_HEADERS_ENV)
            .is_some_and(|value| value.to_lowercase() == "true");
        let url = self
            .litellm_settings
            .anthropic_beta_headers_url
            .or_else(|| lookup.lookup(BETA_HEADERS_URL_ENV))
            .unwrap_or_else(|| DEFAULT_BETA_HEADERS_URL.to_owned());
        Ok(Settings { local_only, url })
    }
}

mod tests;
