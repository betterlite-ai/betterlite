use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use super::context::DispatchContext;

// the provider config call main.py's openrouter and vercel_ai_gateway chat arms make, with the values each arm resolved
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
}

impl CompletionCall {
    // the call both chat arms start from, before they override the fields their own resolution changes
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
        }
    }
}

// the provider config call main.py's two embedding arms make
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
