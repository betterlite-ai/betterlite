use super::HubDispatchError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::str::from_utf8;
pub fn convert<T, U>(value: &T) -> Result<U, HubDispatchError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| HubDispatchError::Encode { source })
}
#[must_use]
pub fn logged_reply(bytes: &[u8]) -> Json {
    serde_json::from_slice::<Json>(bytes).unwrap_or_else(|_error| {
        from_utf8(bytes).map_or(Json::Null, |text| Json::Text(text.to_owned()))
    })
}
#[must_use]
pub fn header_values(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
#[must_use]
pub const fn additional_args(
    api_base: Option<String>,
    headers: Option<IndexMap<String, Json>>,
    body: Option<RawBody>,
    passthrough: IndexMap<String, Json>,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers,
        complete_input_dict: body,
        request_str: None,
        openai_sdk: None,
        passthrough,
    }
}
// LiteLLM logs the opened stream itself as the original response; the stream is not rendered here
pub fn log_opened_stream(
    log: &mut LogContext<'_, '_>,
    messages: &[Message],
    body: Option<RawBody>,
) -> Result<(), HubDispatchError> {
    let opened = PostCall {
        original_response: Json::Null,
        input: convert(messages)?,
        additional_args: additional_args(None, None, body, IndexMap::new()),
    };
    post_call(log.state, &opened, log.env);
    Ok(())
}
