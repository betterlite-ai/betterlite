use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::DispatchError;

pub(super) fn convert<T, U>(value: &T) -> Result<U, DispatchError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| DispatchError::Encode { source })
}

// _complete_hosted_vllm logs the response after the handler; it passes the messages and no additional args
pub fn post(
    log: &mut LogContext<'_, '_>,
    messages: &[Message],
    original_response: Json,
) -> Result<(), DispatchError> {
    let call = PostCall {
        original_response,
        input: convert(messages)?,
        additional_args: AdditionalArgs::default(),
    };
    post_call(log.state, &call, log.env);
    Ok(())
}
