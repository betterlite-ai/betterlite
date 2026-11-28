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

// the first non-empty value of an "a or b or c" chain, since an empty string falls through in LiteLLM
pub fn first_nonempty<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

#[must_use]
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
    pub settings: &'request SectionSettings,
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
    pub settings: &'request SectionSettings,
}
