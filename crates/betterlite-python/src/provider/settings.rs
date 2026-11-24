use secrecy::SecretString;
use serde::Deserialize;

// The module globals get_llm_provider reads at call time: model_list_set and custom_provider_map are its sets, the rest are litellm's credentials.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderSettings {
    pub(crate) custom_provider_map: Vec<CustomProviderEntry>,
    pub(crate) model_list_set: Vec<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) xai_key: Option<SecretString>,
    pub(crate) inception_key: Option<SecretString>,
    pub(crate) lemonade_key: Option<SecretString>,
}

// A custom_provider_map entry is an object with at least a provider name; the handler it also carries stays in Python.
#[derive(Debug, Deserialize)]
pub(crate) struct CustomProviderEntry {
    pub(crate) provider: String,
}
