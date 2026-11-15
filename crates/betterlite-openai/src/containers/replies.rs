use serde::de::DeserializeOwned;

use crate::api::container_contracts::error::ContainerError;

pub fn decode_container_reply<T>(bytes: &[u8]) -> Result<T, ContainerError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| ContainerError::Decode(error.to_string()))
}
