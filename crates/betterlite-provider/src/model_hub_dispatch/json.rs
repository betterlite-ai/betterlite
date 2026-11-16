use super::HubDispatchError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
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
    match serde_json::from_slice::<Json>(bytes) {
        Ok(value) => value,
        Err(_error) => match from_utf8(bytes) {
            Ok(text) => Json::Text(text.to_owned()),
            Err(_error) => Json::Null,
        },
    }
}
#[must_use]
pub fn header_values(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
#[must_use]
pub fn additional_args(
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
