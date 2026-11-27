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

// the regional arms log the error text or the reply they cannot render at this seam, plus the headers GigaChat passed
pub fn post(
    log: &mut CallLogging<'_, '_>,
    messages: &[Message],
    original_response: Json,
    headers: Option<&Headers>,
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
