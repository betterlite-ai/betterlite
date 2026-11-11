use std::collections::BTreeMap;

use betterlite_models::prompt_contracts::{
    AllMessageValues, PromptIgnoreFlags, PromptParams, StandardCallbackDynamicParams,
};
use betterlite_models::prompt_spec::PromptSpec;
use serde::de::DeserializeOwned;

use crate::anthropic_cache_callbacks::hook::AnthropicCacheControlHook;
use crate::cache_accounting::dialect::ModelFacts;
use crate::orchestration::prompt_orchestration::ChatCompletionPromptRequest;

#[derive(Default)]
pub(super) struct Facts {
    breakpoint_flags: BTreeMap<String, bool>,
    providers: BTreeMap<String, String>,
}

impl Facts {
    pub(super) fn supporting_breakpoints(model: &str, provider: &str) -> Self {
        let mut facts = Self::default();
        facts.breakpoint_flags.insert(model.to_owned(), true);
        facts
            .providers
            .insert(model.to_owned(), provider.to_owned());
        facts
    }
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.breakpoint_flags.get(model).copied()
    }

    fn provider(&self, model: &str) -> Option<String> {
        self.providers.get(model).cloned()
    }
}

pub(super) fn hook_with(facts: Facts) -> AnthropicCacheControlHook<Facts> {
    AnthropicCacheControlHook {
        environment: BTreeMap::new(),
        global_api_base: None,
        facts,
    }
}

pub(super) fn plain_hook() -> AnthropicCacheControlHook<Facts> {
    hook_with(Facts::default())
}

pub(super) fn decode<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

pub(super) fn messages(text: &str) -> Vec<AllMessageValues> {
    decode(text)
}

pub(super) fn params(text: &str) -> PromptParams {
    decode(text)
}

pub(super) fn prompt_spec() -> PromptSpec {
    decode(r#"{"prompt_id": "greeting", "litellm_params": {"prompt_integration": "custom"}}"#)
}

pub(super) fn dynamic_params() -> StandardCallbackDynamicParams {
    decode(r#"{"user_id": "user-1"}"#)
}

pub(super) fn empty_dynamic_params() -> StandardCallbackDynamicParams {
    decode("{}")
}

pub(super) fn chat_request<'request>(
    model: &'request str,
    messages: Vec<AllMessageValues>,
    params: PromptParams,
    dynamic: &'request StandardCallbackDynamicParams,
) -> ChatCompletionPromptRequest<'request> {
    ChatCompletionPromptRequest {
        model,
        messages,
        non_default_params: params,
        prompt_id: None,
        prompt_variables: None,
        dynamic_callback_params: dynamic,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
        ignore_flags: PromptIgnoreFlags::default(),
    }
}
