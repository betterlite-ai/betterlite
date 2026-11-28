use betterlite_models::decisions::Json as LogJson;
use betterlite_openai::minimax_chat;

use crate::hosted_platform_dispatch::handler::ChatCompletionHandler;

use super::call::completion_call;
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret};
use super::error::DispatchError;
use super::logging;

// _complete_minimax: the passed key, MINIMAX_API_KEY, then litellm.api_key; the passed base, litellm.api_base, MINIMAX_API_BASE, then the public default; the headers as they came
pub(super) fn minimax<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.environment.minimax_api_key.as_ref(),
        context.settings.api_key.as_ref(),
    ]);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base.as_deref(),
            context.settings.environment.minimax_api_base.as_deref(),
        ])
        .unwrap_or_else(|| String::from(minimax_chat::DEFAULT_API_BASE)),
    );
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    logging::post(log, &call.messages, LogJson::Null, None)?;
    Ok(reply)
}
