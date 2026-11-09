use betterlite_prompts::arguments::json::Json;
use serde::de::DeserializeOwned;

use crate::oci_common::OciError;

pub(crate) fn parse<T>(value: &Json, status: u16, prefix: &str) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    // Json always serializes into a value, and a body that does not fit its type fails below
    let raw = serde_json::to_value(value).unwrap_or_default();
    serde_json::from_value(raw).map_err(|source| OciError {
        status,
        message: format!("{prefix}: {source}"),
    })
}
