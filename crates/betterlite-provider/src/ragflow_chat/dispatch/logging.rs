use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model_hub_dispatch::json;

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

// LiteLLM's post_call carries the error text as the original response and the headers as given
pub(super) fn post(
    log: &mut LogContext<'_, '_>,
    messages: &[Message],
    original_response: Json,
    headers: Option<&BTreeMap<String, String>>,
) -> Result<(), DispatchError> {
    let call = PostCall {
        original_response,
        input: convert(messages)?,
        additional_args: json::additional_args(
            None,
            headers.map(json::header_values),
            None,
            IndexMap::new(),
        ),
    };
    post_call(log.state, &call, log.env);
    Ok(())
}
