use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use super::context::DispatchContext;

// the provider config call main.py's Fireworks, Together, Groq and Cloudflare arms make
#[derive(Debug, Clone)]
pub struct CompletionCall {
    pub model: String,
    pub stream: Option<bool>,
    pub acompletion: bool,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub custom_llm_provider: String,
    pub headers: Option<BTreeMap<String, String>>,
    pub litellm_params: Object,
    pub optional_params: Object,
    pub messages: Vec<Message>,
    pub timeout: Option<Duration>,
    pub provider_config: Option<ChatConfigTag>,
}

impl CompletionCall {
    // the call every chat arm starts from, before it overrides the fields its own resolution changes
    pub(super) fn from_context(context: &DispatchContext<'_>) -> Self {
        Self {
            model: context.model.to_owned(),
            stream: context.stream,
            acompletion: context.acompletion,
            api_base: context.api_base.map(str::to_owned),
            api_key: context.api_key.cloned(),
            custom_llm_provider: context.custom_llm_provider.to_owned(),
            headers: context.headers.cloned(),
            litellm_params: context.litellm_params.clone(),
            optional_params: context.optional_params.clone(),
            messages: context.messages.to_vec(),
            timeout: context.timeout,
            provider_config: context.provider_config,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingCall {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Object,
    pub litellm_params: Object,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout: Duration,
    pub aembedding: bool,
}

// the request fields an operation's config reads; B5834 builds each Perplexity resource config from it
#[derive(Debug, Clone)]
pub struct ResourceCall<'call> {
    pub model: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub headers: Option<&'call BTreeMap<String, String>>,
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
}
