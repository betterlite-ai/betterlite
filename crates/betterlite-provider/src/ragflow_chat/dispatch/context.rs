use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;

// LiteLLM's module-level SDK settings: api_base, api_key and headers, which the dispatch falls back to
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchGlobals<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
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
    pub globals: DispatchGlobals<'request>,
    pub secrets: &'request SecretLookup,
    pub environment: &'request BTreeMap<String, String>,
}
