use secrecy::ExposeSecret as _;

use crate::a2a_chat::registry::{RegistryInput, resolve_agent_config};

use super::call::CompletionCall;
use super::context::{DispatchContext, headers_or_global};
use super::environment::environment_secret;
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;

pub const API_BASE_ENV: &str = "A2A_API_BASE";

// LiteLLM's _complete_a2a: the registry agent first, then litellm.api_base, then A2A_API_BASE
pub fn complete_a2a<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let mut optional_params = context.optional_params.clone();
    let resolved = resolve_agent_config(
        &RegistryInput {
            agent_name: context.model,
            api_base: context.api_base,
            api_key: context.api_key,
            headers: context.headers,
        },
        &mut optional_params,
        context.registry,
        context.entra,
    )?;
    let base = api_base(context, resolved.api_base)?;
    Ok(handler.complete(&CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: base,
        api_key: resolved.api_key,
        custom_llm_provider: context.custom_llm_provider.to_owned(),
        headers: headers_or_global(resolved.headers, context.globals.headers),
        litellm_params: context.litellm_params.clone(),
        optional_params,
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: context.provider_config,
    })?)
}

fn api_base(
    context: &DispatchContext<'_>,
    resolved: Option<String>,
) -> Result<String, DispatchError> {
    if let Some(base) = resolved.filter(|base| !base.is_empty()) {
        return Ok(base);
    }
    if let Some(base) = context.globals.api_base.filter(|base| !base.is_empty()) {
        return Ok(String::from(base));
    }
    let base = environment_secret(context.secrets, context.environment, API_BASE_ENV)?
        .ok_or(DispatchError::MissingA2aApiBase)?;
    Ok(base.expose_secret().to_owned())
}
