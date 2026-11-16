use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model_hub_dispatch::json;

use super::context::CallLogging;
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

fn args(headers: Option<&Headers>) -> AdditionalArgs {
    json::additional_args(
        None,
        headers.map(json::header_values),
        None,
        IndexMap::new(),
    )
}

// LiteLLM logs the handler error's text as the original response and keeps the headers as given
pub fn post_failure(
    log: &mut CallLogging<'_, '_>,
    messages: &[Message],
    error: &str,
    headers: Option<&Headers>,
) -> Result<(), DispatchError> {
    let call = PostCall {
        original_response: Json::Text(error.to_owned()),
        input: convert(messages)?,
        additional_args: args(headers),
    };
    post_call(log.state, &call, log.env);
    Ok(())
}

// LiteLLM logs the opened stream object itself; a Rust stream has no JSON form, so the record carries null
pub fn post_opened_stream(
    log: &mut CallLogging<'_, '_>,
    messages: &[Message],
    headers: Option<&Headers>,
) -> Result<(), DispatchError> {
    let call = PostCall {
        original_response: Json::Null,
        input: convert(messages)?,
        additional_args: args(headers),
    };
    post_call(log.state, &call, log.env);
    Ok(())
}
