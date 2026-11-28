use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::hosted_platform_keys::Settings as SectionSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::{ExposeSecret as _, SecretString};

// the hosted_platform_keys section's values borrowed for one call
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
    pub groq_key: Option<&'settings SecretString>,
    pub cloudflare_api_key: Option<&'settings SecretString>,
    pub env_groq_api_key: Option<&'settings SecretString>,
    pub env_groq_api_base: Option<&'settings str>,
    pub env_cloudflare_api_key: Option<&'settings SecretString>,
    pub env_cloudflare_api_base: Option<&'settings str>,
    pub env_cloudflare_account_id: Option<&'settings str>,
    pub env_fireworks_ai_api_key: Option<&'settings SecretString>,
}

impl<'settings> From<&'settings SectionSettings> for DispatchSettings<'settings> {
    fn from(settings: &'settings SectionSettings) -> Self {
        let environment = &settings.environment;
        Self {
            api_base: settings.api_base.as_deref(),
            api_key: settings.api_key.as_ref(),
            headers: settings.headers.as_ref(),
            groq_key: settings.groq_key.as_ref(),
            cloudflare_api_key: settings.cloudflare_api_key.as_ref(),
            env_groq_api_key: environment.groq_api_key.as_ref(),
            env_groq_api_base: environment.groq_api_base.as_deref(),
            env_cloudflare_api_key: environment.cloudflare_api_key.as_ref(),
            env_cloudflare_api_base: environment.cloudflare_api_base.as_deref(),
            env_cloudflare_account_id: environment.cloudflare_account_id.as_deref(),
            env_fireworks_ai_api_key: environment.fireworks_ai_api_key.as_ref(),
        }
    }
}

// the first non-empty value of an "a or b or c" chain, since an empty string falls through in LiteLLM
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

pub struct DispatchContext<'request> {
    pub acompletion: bool,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub custom_llm_provider: &'request str,
    // litellm.GroqChatConfig.get_config(), which holds only Python SDK class defaults and is empty in the proxy
    pub groq_config: &'request Object,
    pub headers: Option<&'request BTreeMap<String, String>>,
    pub litellm_params: &'request Object,
    pub messages: &'request [Message],
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub provider_config: Option<ChatConfigTag>,
    pub stream: Option<bool>,
    pub timeout: Option<Duration>,
    pub settings: DispatchSettings<'request>,
}
