use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::EmbeddingIoError;

pub(super) fn parse_one<T>(object: &Object) -> Result<T, EmbeddingIoError>
where
    T: DeserializeOwned,
{
    let value =
        serde_json::to_value(object).map_err(|source| EmbeddingIoError::Encode { source })?;
    serde_json::from_value(value).map_err(|source| EmbeddingIoError::Params { source })
}

pub(super) fn parse_list<T>(objects: &[Object]) -> Result<Vec<T>, EmbeddingIoError>
where
    T: DeserializeOwned,
{
    let mut parsed = Vec::with_capacity(objects.len());
    for object in objects {
        parsed.push(parse_one(object)?);
    }
    Ok(parsed)
}

pub(super) fn encode_json<T>(value: &T) -> Result<Json, EmbeddingIoError>
where
    T: Serialize,
{
    let text =
        serde_json::to_string(value).map_err(|source| EmbeddingIoError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| EmbeddingIoError::Params { source })
}
