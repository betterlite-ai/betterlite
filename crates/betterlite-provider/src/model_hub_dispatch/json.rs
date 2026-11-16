use super::HubDispatchError;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::str::from_utf8;
pub fn convert<T, U>(value: &T) -> Result<U, HubDispatchError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    let raw = serde_json::to_value(value).map_err(|source| HubDispatchError::Encode { source })?;
    serde_json::from_value(raw).map_err(|source| HubDispatchError::Encode { source })
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
