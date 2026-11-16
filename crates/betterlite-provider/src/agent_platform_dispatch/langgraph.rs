use secrecy::ExposeSecret;

use crate::langgraph_chat::config;

use super::call::{ResolvedRoute, build_call};
use super::context::{DispatchContext, headers_or_global};
use super::environment::environment_secret;
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;

// LiteLLM's _complete_langgraph: the provider info pair, the headers fallback and no provider_config
pub fn complete_langgraph<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let env_base = environment_secret(context.secrets, context.environment, config::API_BASE_ENV)?;
    let env_key = environment_secret(context.secrets, context.environment, config::API_KEY_ENV)?;
    let base = context
        .api_base
        .filter(|base| !base.is_empty())
        .or(context.globals.api_base);
    let key = context
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(context.globals.api_key);
    let info = config::provider_info(
        base,
        key,
        env_base.as_ref().map(ExposeSecret::expose_secret),
        env_key.as_ref(),
    );
    let route = ResolvedRoute {
        api_base: Some(info.api_base),
        api_key: info.api_key,
        headers: headers_or_global(context.headers.cloned(), context.globals.headers),
        optional_params: context.optional_params.clone(),
        provider_config: None,
    };
    let reply = handler.complete(&build_call(context, route))?;
    Ok(reply)
}
