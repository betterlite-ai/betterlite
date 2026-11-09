use betterlite_prompts::arguments::json::Json;
use serde::de::DeserializeOwned;

use crate::oci_common::OciError;

pub(crate) fn parse_response<T>(body: &Json, name: &str, status: u16) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    let bytes = serde_json::to_vec(body).map_err(|source| OciError {
        status,
        message: format!("Response cannot be casted to {name}: {source}"),
    })?;
    serde_json::from_slice(&bytes).map_err(|source| OciError {
        status,
        message: format!("Response cannot be casted to {name}: {source}"),
    })
}

pub(crate) fn parse_chunk<T>(chunk: &Json, name: &str) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    let bytes = serde_json::to_vec(chunk).map_err(|source| OciError {
        status: 500,
        message: format!("Chunk cannot be parsed as {name}: {source}"),
    })?;
    serde_json::from_slice(&bytes).map_err(|source| OciError {
        status: 500,
        message: format!("Chunk cannot be parsed as {name}: {source}"),
    })
}
