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
            // get_secret_str answers the raw string, and a later `or` treats an empty one as unset
            azure_api_type: lookup
                .lookup("AZURE_API_TYPE")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_AZURE_API_TYPE.to_owned()),
            azure_api_base: lookup
                .lookup("AZURE_API_BASE")
                .filter(|value| !value.is_empty()),
            azure_api_version: lookup
                .lookup("AZURE_API_VERSION")
                .filter(|value| !value.is_empty()),
            azure_openai_api_key: lookup
                .lookup("AZURE_OPENAI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            azure_api_key: lookup
                .lookup("AZURE_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            azure_ad_token: lookup
                .lookup("AZURE_AD_TOKEN")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            azure_ai_api_base: lookup
                .lookup("AZURE_AI_API_BASE")
                .filter(|value| !value.is_empty()),
            azure_ai_api_key: lookup
                .lookup("AZURE_AI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
