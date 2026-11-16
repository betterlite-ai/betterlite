use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use super::context::DispatchContext;

// the call a platform dispatch hands the shared HTTP completion handler
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

// what the provider's config resolved, before it becomes a call
pub struct ResolvedRoute {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<BTreeMap<String, String>>,
    pub optional_params: Object,
    pub provider_config: Option<ChatConfigTag>,
}

#[must_use]
pub fn build_call(context: &DispatchContext<'_>, route: ResolvedRoute) -> CompletionCall {
    CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: route.api_base,
        api_key: route.api_key,
        custom_llm_provider: context.custom_llm_provider.to_owned(),
        headers: route.headers,
        litellm_params: context.litellm_params.clone(),
        optional_params: route.optional_params,
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: route.provider_config,
    }
}
