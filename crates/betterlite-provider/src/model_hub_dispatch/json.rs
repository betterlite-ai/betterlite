use std::str::from_utf8;

use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::HubDispatchError;

// Json and serde_json::Value hold the same shapes, so serde moves values between them without failing
pub fn convert<T, U>(value: &T) -> Result<U, HubDispatchError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    let raw = serde_json::to_value(value).map_err(|source| HubDispatchError::Encode { source })?;
    serde_json::from_value(raw).map_err(|source| HubDispatchError::Encode { source })
}

// LiteLLM hands the callbacks the live response; the copy betterlite keeps is the parsed body, or its text when it is not JSON
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
