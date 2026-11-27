use betterlite_prompts::arguments::json::Object;

use crate::hosted_platform_dispatch::call::{CompletionCall, EmbeddingCall};

use super::context::{DispatchContext, EmbeddingContext};

// the call every regional chat arm starts from, before it overrides the fields its own resolution changes
pub fn completion_call(context: &DispatchContext<'_>) -> CompletionCall {
    CompletionCall {
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

// the call every regional embedding arm starts from; each arm sets its own litellm_params and headers
pub fn embedding_call(context: &EmbeddingContext<'_>) -> EmbeddingCall {
    EmbeddingCall {
        model: context.model.to_owned(),
        input: context.input.clone(),
        optional_params: context.optional_params.clone(),
        litellm_params: Object::new(),
        api_key: context.api_key.cloned(),
        api_base: context.api_base.map(str::to_owned),
        headers: context.extra_headers.cloned(),
        timeout: context.timeout,
        aembedding: context.aembedding,
    }
}
