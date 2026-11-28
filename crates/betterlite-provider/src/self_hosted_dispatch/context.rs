use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::compatible_embedding_settings;
use betterlite_config::sections::self_hosted_dispatch::Settings as SectionSettings;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::{ExposeSecret as _, SecretString};

use crate::ollama_generate::config::DEFAULT_API_BASE as OLLAMA_DEFAULT_API_BASE;

// the config section's values borrowed for one call
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchSettings<'settings> {
    pub hosted_vllm_api_base: Option<&'settings str>,
    pub hosted_vllm_api_key: Option<&'settings SecretString>,
    pub ollama_api_base: Option<&'settings str>,
    pub ollama_api_key: Option<&'settings SecretString>,
    pub lemonade_api_key: Option<&'settings SecretString>,
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub ollama_key: Option<&'settings SecretString>,
    pub lemonade_key: Option<&'settings SecretString>,
}

impl<'settings> From<&'settings SectionSettings> for DispatchSettings<'settings> {
    fn from(settings: &'settings SectionSettings) -> Self {
        Self {
            hosted_vllm_api_base: settings.hosted_vllm_api_base.as_deref(),
            hosted_vllm_api_key: settings.hosted_vllm_api_key.as_ref(),
            ollama_api_base: settings.ollama_api_base.as_deref(),
            ollama_api_key: settings.ollama_api_key.as_ref(),
            lemonade_api_key: settings.lemonade_api_key.as_ref(),
            api_base: settings.api_base.as_deref(),
            api_key: settings.api_key.as_ref(),
            ollama_key: settings.ollama_key.as_ref(),
            lemonade_key: settings.lemonade_key.as_ref(),
        }
    }
}

// the first non-empty value of a "a or b or c" chain, since an empty string falls through in LiteLLM
pub(super) fn first_nonempty<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(super) fn first_secret<const N: usize>(
    candidates: [Option<&SecretString>; N],
) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}

// both ollama arms and the ollama embedding arm read litellm.api_base, then the passed base, then OLLAMA_API_BASE
pub(super) fn ollama_api_base(settings: DispatchSettings<'_>, api_base: Option<&str>) -> String {
    first_nonempty([settings.api_base, api_base, settings.ollama_api_base])
        .unwrap_or_else(|| String::from(OLLAMA_DEFAULT_API_BASE))
}

pub struct DispatchContext<'request> {
    pub acompletion: bool,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub custom_llm_provider: &'request str,
    pub headers: Option<&'request BTreeMap<String, String>>,
    pub litellm_params: &'request Object,
    pub messages: &'request [Message],
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub stream: Option<bool>,
    pub timeout: Option<Duration>,
    pub settings: DispatchSettings<'request>,
}

pub struct EmbeddingContext<'request> {
    pub settings: DispatchSettings<'request>,
    pub environment: &'request dyn EnvironmentLookup,
    pub compatible: &'request compatible_embedding_settings::Settings,
    pub globals: &'request compatible_embedding_settings::Globals,
}
