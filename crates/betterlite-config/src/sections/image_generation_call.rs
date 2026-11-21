mod tests;

use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

// LiteLLM's get_secret_str("AZURE_API_TYPE") or "azure": the local value is read and never used
const DEFAULT_AZURE_API_TYPE: &str = "azure";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
}

// The litellm module globals and the Azure environment the image generation fallback chains read.
#[derive(Debug, Clone)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub azure_api_type: String,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub azure_ai_api_base: Option<String>,
    pub azure_ai_api_key: Option<SecretString>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_base: None,
            api_version: None,
            api_key: None,
            azure_key: None,
            azure_api_type: DEFAULT_AZURE_API_TYPE.to_owned(),
            azure_api_base: None,
            azure_api_version: None,
            azure_openai_api_key: None,
            azure_api_key: None,
            azure_ad_token: None,
            azure_ai_api_base: None,
            azure_ai_api_key: None,
        }
    }
}

// get_secret_str answers the raw string, and a later `or` treats an empty one as unset
fn text<L: EnvironmentLookup>(lookup: &L, name: &str) -> Option<String> {
    lookup.lookup(name).filter(|value| !value.is_empty())
}

fn secret<L: EnvironmentLookup>(lookup: &L, name: &str) -> Option<SecretString> {
    text(lookup, name).map(SecretString::from)
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let litellm_settings = self.settings;
        Ok(Settings {
            api_base: litellm_settings
                .api_base
                .map(|value| resolve_string(value, "litellm_settings.api_base", lookup))
                .transpose()?,
            api_version: litellm_settings
                .api_version
                .map(|value| resolve_string(value, "litellm_settings.api_version", lookup))
                .transpose()?,
            api_key: resolve_secret(litellm_settings.api_key, "litellm_settings.api_key", lookup)?,
            azure_key: resolve_secret(
                litellm_settings.azure_key,
                "litellm_settings.azure_key",
                lookup,
            )?,
            azure_api_type: text(lookup, "AZURE_API_TYPE")
                .unwrap_or_else(|| DEFAULT_AZURE_API_TYPE.to_owned()),
            azure_api_base: text(lookup, "AZURE_API_BASE"),
            azure_api_version: text(lookup, "AZURE_API_VERSION"),
            azure_openai_api_key: secret(lookup, "AZURE_OPENAI_API_KEY"),
            azure_api_key: secret(lookup, "AZURE_API_KEY"),
            azure_ad_token: secret(lookup, "AZURE_AD_TOKEN"),
            azure_ai_api_base: text(lookup, "AZURE_AI_API_BASE"),
            azure_ai_api_key: secret(lookup, "AZURE_AI_API_KEY"),
        })
    }
}
