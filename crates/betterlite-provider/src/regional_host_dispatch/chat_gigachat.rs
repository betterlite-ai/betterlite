use betterlite_models::decisions::Json as LogJson;

use crate::agent_platform_dispatch::context::headers_or_global;
use crate::hosted_platform_dispatch::handler::ChatCompletionHandler;

use super::call::completion_call;
use super::context::{CallLogging, DispatchContext, first_secret};
use super::error::DispatchError;
use super::logging;

// _complete_gigachat: the passed key, litellm.api_key, litellm.gigachat_key, GIGACHAT_API_KEY and GIGACHAT_CREDENTIALS; the passed headers, litellm.headers, then an empty map; the base as it came
pub(super) fn gigachat<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key.as_ref(),
        context.settings.gigachat_key.as_ref(),
        context.settings.environment.gigachat_api_key.as_ref(),
        context.settings.environment.gigachat_credentials.as_ref(),
    ]);
    let headers = headers_or_global(context.headers.cloned(), context.settings.headers.as_ref())
        .unwrap_or_default();
    call.headers = Some(headers.clone());
    match handler.complete(&call) {
        Ok(reply) => Ok(reply),
        Err(error) => {
            logging::post(
                log,
                &call.messages,
                LogJson::Text(error.to_string()),
                Some(&headers),
            )?;
            Err(DispatchError::Handler(error))
        }
    }
}
