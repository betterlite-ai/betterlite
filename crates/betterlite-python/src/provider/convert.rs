use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::provider::error::ProviderError;

// get_llm_provider(model, custom_llm_provider=None, api_base=None, api_key=None)
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct LookupRequest {
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
}

// _get_openai_compatible_provider_info(model, api_base, api_key, dynamic_api_key)
#[derive(Debug, Deserialize)]
pub(crate) struct CompatibleRequest {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) dynamic_api_key: Option<SecretString>,
}

// The two handlers and declared_authenticating_provider take a model and a declared provider.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct HandlerRequest {
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LookupResult {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
}

impl LookupResult {
    pub(crate) fn to_json(&self) -> Result<String, ProviderError> {
        encode(&(
            self.model.as_str(),
            self.provider.as_str(),
            self.api_key
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            self.api_base.as_deref(),
        ))
    }
}

pub(crate) fn parse<T>(text: &str) -> Result<T, ProviderError>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ProviderError::Json { source }),
    }
}

pub(crate) fn encode<T>(value: &T) -> Result<String, ProviderError>
where
    T: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => Ok(text),
        Err(source) => Err(ProviderError::Encode { source }),
    }
}
