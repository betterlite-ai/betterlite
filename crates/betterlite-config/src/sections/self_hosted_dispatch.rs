use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

// main.py's self-hosted arms: the provider variables through get_secret_str, the SDK globals through litellm
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    ollama_key: Option<SecretString>,
    lemonade_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub hosted_vllm_api_base: Option<String>,
    pub hosted_vllm_api_key: Option<SecretString>,
    pub ollama_api_base: Option<String>,
    pub ollama_api_key: Option<SecretString>,
    pub lemonade_api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ollama_key: Option<SecretString>,
    pub lemonade_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // get_secret_str and os.environ.get read the provider variables; an empty one counts as unset
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(Settings {
            hosted_vllm_api_base: read("HOSTED_VLLM_API_BASE"),
            hosted_vllm_api_key: read("HOSTED_VLLM_API_KEY").map(SecretString::from),
            ollama_api_base: read("OLLAMA_API_BASE"),
            ollama_api_key: read("OLLAMA_API_KEY").map(SecretString::from),
            lemonade_api_key: read("LEMONADE_API_KEY").map(SecretString::from),
            api_base: match self.litellm_settings.api_base {
                Some(text) => Some(resolve_string(text, "litellm_settings.api_base", lookup)?),
                None => None,
            },
            api_key: resolve_secret(
                self.litellm_settings.api_key,
                "litellm_settings.api_key",
                lookup,
            )?,
            ollama_key: resolve_secret(
                self.litellm_settings.ollama_key,
                "litellm_settings.ollama_key",
                lookup,
            )?,
            lemonade_key: resolve_secret(
                self.litellm_settings.lemonade_key,
                "litellm_settings.lemonade_key",
                lookup,
            )?,
        })
    }
}
