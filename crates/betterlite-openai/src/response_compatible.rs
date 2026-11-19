use std::collections::BTreeMap;

use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};

use crate::response_params::capabilities::{ProviderCapabilities, provider_capabilities};

mod tests;

pub const RESPONSES_PATH: &str = "/responses";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CompatibleConfigError {
    #[error("api_base is required for openai_like provider")]
    MissingApiBase,
}

// LiteLLM's OpenAILikeResponsesConfig: identity, headers and URL, over B506/B507/B508/B509's inherited operations
#[derive(Debug, Clone, Default)]
pub struct OpenAiLikeResponsesConfig {
    pub settings: OpenAiLikeSettings,
}

impl OpenAiLikeResponsesConfig {
    // LiteLLM's OpenAILikeResponsesConfig identity: the fallback always reports the OpenAI-like provider
    pub const IDENTITY: ProviderCapabilities = provider_capabilities(ProviderId::OpenaiLike);

    // LiteLLM's validate_environment override: a nonempty key supplies the only header this class adds
    #[must_use]
    pub fn validate_environment(
        &self,
        mut headers: BTreeMap<String, String>,
        api_key: Option<&SecretString>,
    ) -> BTreeMap<String, String> {
        let (_base, key) = self.settings.resolve(None, api_key);
        let plain = key.expose_secret();
        if !plain.is_empty() {
            headers.insert(String::from("Authorization"), format!("Bearer {plain}"));
        }
        headers
    }

    // LiteLLM's get_complete_url override: the base comes from the setting or the environment, never a default
    pub fn get_complete_url(
        &self,
        api_base: Option<&str>,
    ) -> Result<String, CompatibleConfigError> {
        let (base, _key) = self.settings.resolve(api_base, None);
        let Some(base) = base else {
            return Err(CompatibleConfigError::MissingApiBase);
        };
        Ok(format!("{}{RESPONSES_PATH}", base.trim_end_matches('/')))
    }
}
