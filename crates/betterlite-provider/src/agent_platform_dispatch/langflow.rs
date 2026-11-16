use secrecy::ExposeSecret as _;

use crate::langflow_chat::config;

use super::call::CompletionCall;
use super::context::{DispatchContext, headers_or_global};
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;

// LiteLLM's _complete_langflow: the provider info pair, the headers fallback and no provider_config
pub fn complete_langflow<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let base = context
        .api_base
        .filter(|base| !base.is_empty())
        .or(context.globals.api_base);
    let key = context
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(context.globals.api_key);
    let info = config::provider_info(base, key, context.secrets, context.environment)?;
    Ok(handler.complete(&CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: info.api_base,
        api_key: info.api_key,
        custom_llm_provider: context.custom_llm_provider.to_owned(),
        headers: headers_or_global(context.headers.cloned(), context.globals.headers),
        litellm_params: context.litellm_params.clone(),
        optional_params: context.optional_params.clone(),
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: None,
    })?)
}
