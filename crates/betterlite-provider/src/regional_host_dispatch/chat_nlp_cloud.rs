use betterlite_models::decisions::Json as LogJson;
use betterlite_prompts::arguments::json::Json;

use crate::hosted_platform_dispatch::handler::ChatCompletionHandler;

use super::call::completion_call;
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret};
use super::error::DispatchError;
use super::logging;

const NLP_CLOUD_DEFAULT_API_BASE: &str = "https://api.nlpcloud.io/v1/gpu/";

// _complete_nlp_cloud: the passed key, nlp_cloud_key, NLP_CLOUD_API_KEY, then litellm.api_key; the passed base, litellm.api_base, NLP_CLOUD_API_BASE, then the GPU default
pub(super) fn nlp_cloud<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.nlp_cloud_key.as_ref(),
        context.settings.environment.nlp_cloud_api_key.as_ref(),
        context.settings.api_key.as_ref(),
    ]);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base.as_deref(),
            context.settings.environment.nlp_cloud_api_base.as_deref(),
        ])
        .unwrap_or_else(|| String::from(NLP_CLOUD_DEFAULT_API_BASE)),
    );
    call.headers = None;
    call.provider_config = None;
    // LiteLLM wraps the reply when optional_params carries stream true, and logs when it streams or the call is async
    let streaming = matches!(
        context.optional_params.get("stream"),
        Some(Json::Bool(true))
    );
    call.stream = Some(streaming);
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    if call.stream == Some(true) || context.acompletion {
        logging::post(log, &call.messages, LogJson::Null, None)?;
    }
    Ok(reply)
}
