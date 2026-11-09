use betterlite_prompts::arguments::json::Json;
use serde::de::DeserializeOwned;

use crate::oci_common::OciError;

pub(crate) fn parse<T>(value: &Json, status: u16, prefix: &str) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(|source| OciError {
        status,
        message: format!("{prefix}: {source}"),
    })?;
    serde_json::from_slice(&bytes).map_err(|source| OciError {
        status,
        message: format!("{prefix}: {source}"),
    })
}
