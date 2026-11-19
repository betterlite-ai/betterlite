mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;

pub const ALEPH_ALPHA_API_KEY_ENV: &str = "ALEPH_ALPHA_API_KEY";
pub const ALEPHALPHA_API_KEY_ENV: &str = "ALEPHALPHA_API_KEY";
pub const ALEPH_ALPHA_API_BASE_ENV: &str = "ALEPH_ALPHA_API_BASE";
// main.py's last link of the base chain
pub const DEFAULT_ALEPH_ALPHA_BASE: &str = "https://api.aleph-alpha.com/complete";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    settings: RawSettings,
}

// custom_provider_map is deliberately absent: the loader has to keep refusing it until B10006 reads it
#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    aleph_alpha_key: Option<SecretString>,
}

// Every source main.py's two chains read, in the order it reads them; the resolve methods apply that order.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub aleph_alpha_key: Option<SecretString>,
    pub aleph_alpha_api_key: Option<SecretString>,
    pub alephalpha_api_key: Option<SecretString>,
    pub aleph_alpha_api_base: Option<String>,
}

impl Settings {
    // api_key, litellm.aleph_alpha_key, ALEPH_ALPHA_API_KEY, ALEPHALPHA_API_KEY, litellm.api_key
    #[must_use]
    pub fn resolve_aleph_alpha_key(
        &self,
        supplied: Option<&SecretString>,
        global: Option<&SecretString>,
    ) -> Option<SecretString> {
        // LiteLLM's "or" chain skips an empty value, so an empty spelling leaves the next source to answer
        let supplied = supplied.filter(|key| !key.expose_secret().is_empty());
        let yaml = self
            .aleph_alpha_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let first_env = self
            .aleph_alpha_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let second_env = self
            .alephalpha_api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let global = global.filter(|key| !key.expose_secret().is_empty());
        supplied
            .or(yaml)
            .or(first_env)
            .or(second_env)
            .or(global)
            .cloned()
    }

    // api_base, litellm.api_base, ALEPH_ALPHA_API_BASE, then the provider's own host
    #[must_use]
    pub fn resolve_aleph_alpha_base(&self, supplied: Option<&str>, global: Option<&str>) -> String {
        let supplied = supplied.filter(|text| !text.is_empty());
        let global = global.filter(|text| !text.is_empty());
        let environment = self
            .aleph_alpha_api_base
            .as_deref()
            .filter(|text| !text.is_empty());
        supplied
            .or(global)
            .or(environment)
            .map_or_else(|| String::from(DEFAULT_ALEPH_ALPHA_BASE), str::to_owned)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            aleph_alpha_key: resolve_secret(
                self.settings.aleph_alpha_key,
                "litellm_settings.aleph_alpha_key",
                lookup,
            )?,
            aleph_alpha_api_key: lookup
                .lookup(ALEPH_ALPHA_API_KEY_ENV)
                .map(SecretString::from),
            alephalpha_api_key: lookup
                .lookup(ALEPHALPHA_API_KEY_ENV)
                .map(SecretString::from),
            aleph_alpha_api_base: lookup.lookup(ALEPH_ALPHA_API_BASE_ENV),
        })
    }
}
