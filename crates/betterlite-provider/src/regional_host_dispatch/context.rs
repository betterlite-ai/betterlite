use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::{ExposeSecret as _, SecretString};

// the regional_host_keys section's values borrowed for one call
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
    pub gigachat_key: Option<&'settings SecretString>,
    pub nlp_cloud_key: Option<&'settings SecretString>,
    pub env_gigachat_api_key: Option<&'settings SecretString>,
    pub env_gigachat_credentials: Option<&'settings SecretString>,
    pub env_nlp_cloud_api_key: Option<&'settings SecretString>,
    pub env_nlp_cloud_api_base: Option<&'settings str>,
    pub env_minimax_api_key: Option<&'settings SecretString>,
    pub env_minimax_api_base: Option<&'settings str>,
    pub env_ark_api_key: Option<&'settings SecretString>,
    pub env_volcengine_api_key: Option<&'settings SecretString>,
    pub env_dashscope_api_key: Option<&'settings SecretString>,
    pub env_qwencloud_key: Option<&'settings SecretString>,
    pub env_qwen_ai_platform_key: Option<&'settings SecretString>,
}

impl<'settings> From<&'settings SectionSettings> for DispatchSettings<'settings> {
    fn from(settings: &'settings SectionSettings) -> Self {
        let environment = &settings.environment;
        Self {
            api_base: settings.api_base.as_deref(),
            api_key: settings.api_key.as_ref(),
            headers: settings.headers.as_ref(),
            gigachat_key: settings.gigachat_key.as_ref(),
            nlp_cloud_key: settings.nlp_cloud_key.as_ref(),
            env_gigachat_api_key: environment.gigachat_api_key.as_ref(),
            env_gigachat_credentials: environment.gigachat_credentials.as_ref(),
            env_nlp_cloud_api_key: environment.nlp_cloud_api_key.as_ref(),
            env_nlp_cloud_api_base: environment.nlp_cloud_api_base.as_deref(),
            env_minimax_api_key: environment.minimax_api_key.as_ref(),
            env_minimax_api_base: environment.minimax_api_base.as_deref(),
            env_ark_api_key: environment.ark_api_key.as_ref(),
            env_volcengine_api_key: environment.volcengine_api_key.as_ref(),
            env_dashscope_api_key: environment.dashscope_api_key.as_ref(),
            env_qwencloud_key: environment.qwencloud_key.as_ref(),
            env_qwen_ai_platform_key: environment.qwen_ai_platform_key.as_ref(),
        }
    }
}

// the first non-empty value of an "a or b or c" chain, since an empty string falls through in LiteLLM
pub fn first_nonempty<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn first_secret<const N: usize>(
    candidates: [Option<&SecretString>; N],
) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}

pub struct CallLogging<'state, 'env> {
    pub state: &'state mut LoggingState,
    pub env: &'state PrePostEnv<'env>,
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
    pub provider_config: Option<ChatConfigTag>,
    pub stream: Option<bool>,
    pub timeout: Option<Duration>,
    pub settings: DispatchSettings<'request>,
}

pub struct EmbeddingContext<'request> {
    pub aembedding: bool,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub input: &'request EmbeddingInput,
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub ssl_verify: Option<bool>,
    pub timeout: Duration,
    pub settings: DispatchSettings<'request>,
}
