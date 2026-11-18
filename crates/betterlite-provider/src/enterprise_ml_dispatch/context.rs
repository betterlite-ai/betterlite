use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::data_cloud_dispatch::call::CompletionCall;
use crate::data_cloud_dispatch::context::DispatchContext;

use super::call::SagemakerRequest;

// the call fields the three arms read outside the shared call
pub struct ChatContext<'request> {
    pub base: DispatchContext<'request>,
    pub sagemaker: SagemakerRequest<'request>,
}

// the call fields an arm replaces before the shared handler reads the rest of the context
pub struct ReplacedCall<'request> {
    pub optional_params: Object,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub provider: &'request str,
    pub headers: Option<BTreeMap<String, String>>,
}

// the handler call every passthrough arm hands over, with the fields it replaced
#[must_use]
pub fn handler_call(context: &DispatchContext<'_>, replaced: ReplacedCall<'_>) -> CompletionCall {
    CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: replaced.api_base,
        api_key: replaced.api_key,
        custom_llm_provider: replaced.provider.to_owned(),
        headers: replaced.headers,
        litellm_params: context.litellm_params.clone(),
        optional_params: replaced.optional_params,
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: context.provider_config,
    }
}

// the context fields an arm passes unchanged
#[must_use]
pub fn passthrough(context: &DispatchContext<'_>, provider: &str) -> CompletionCall {
    handler_call(
        context,
        ReplacedCall {
            optional_params: context.optional_params.clone(),
            api_key: context.api_key.cloned(),
            api_base: context.api_base.map(str::to_owned),
            provider,
            headers: context.headers.cloned(),
        },
    )
}
