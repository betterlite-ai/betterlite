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

// the handler call every passthrough arm hands over, with the fields it replaced
pub fn handler_call(
    context: &DispatchContext<'_>,
    optional_params: Object,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    provider: &str,
    headers: Option<BTreeMap<String, String>>,
) -> CompletionCall {
    CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base,
        api_key,
        custom_llm_provider: provider.to_owned(),
        headers,
        litellm_params: context.litellm_params.clone(),
        optional_params,
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: context.provider_config,
    }
}

// the context fields an arm passes unchanged
pub fn passthrough(context: &DispatchContext<'_>, provider: &str) -> CompletionCall {
    handler_call(
        context,
        context.optional_params.clone(),
        context.api_key.cloned(),
        context.api_base.map(str::to_owned),
        provider,
        context.headers.cloned(),
    )
}
