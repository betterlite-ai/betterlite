use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;

use super::error::DispatchError;

// both arms log post_call only after the call succeeds; LiteLLM passes openai.api_key there, which has no betterlite counterpart
pub(super) fn post(
    log: &mut LogContext<'_, '_>,
    messages: &[Message],
    original_response: Json,
) -> Result<(), DispatchError> {
    let input = serde_json::to_value(messages)
        .and_then(serde_json::from_value)
        .map_err(|source| DispatchError::Encode { source })?;
    let call = PostCall {
        original_response,
        input,
        additional_args: AdditionalArgs::default(),
    };
    post_call(log.state, &call, log.env);
    Ok(())
}
