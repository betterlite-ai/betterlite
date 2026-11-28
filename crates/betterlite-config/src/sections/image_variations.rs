mod tests;

use crate::env_integer::timeout::RequestTimeout;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
}

// The litellm module globals and environment the image variation credentials chain reads.
#[derive(Debug, Clone)]
pub struct Settings {
    pub request_timeout_seconds: f64,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub topaz_api_key: Option<SecretString>,
    pub topaz_api_base: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            request_timeout_seconds: RequestTimeout::DEFAULT_SECONDS,
            api_base: None,
            api_key: None,
            openai_key: None,
            openai_base_url: None,
            openai_api_base: None,
            openai_api_key: None,
            topaz_api_key: None,
            topaz_api_base: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let litellm_settings = self.settings;
        let environment_text = lookup.lookup("REQUEST_TIMEOUT").unwrap_or_default();
        let request_timeout_seconds =
            betterlite_env::number::<f64>("REQUEST_TIMEOUT", &environment_text)?
                .unwrap_or(RequestTimeout::DEFAULT_SECONDS);
        let settings = Settings {
            request_timeout_seconds,
            api_base: litellm_settings
                .api_base
                .map(|text| resolve_string(text, "litellm_settings.api_base", lookup))
                .transpose()?,
            api_key: resolve_secret(litellm_settings.api_key, "litellm_settings.api_key", lookup)?,
            openai_key: resolve_secret(
                litellm_settings.openai_key,
                "litellm_settings.openai_key",
                lookup,
            )?,
            openai_base_url: lookup.lookup("OPENAI_BASE_URL"),
            openai_api_base: lookup.lookup("OPENAI_API_BASE"),
            openai_api_key: lookup.lookup("OPENAI_API_KEY").map(SecretString::from),
            topaz_api_key: lookup.lookup("TOPAZ_API_KEY").map(SecretString::from),
            topaz_api_base: lookup.lookup("TOPAZ_API_BASE"),
        };
        Ok(settings)
    }
}
