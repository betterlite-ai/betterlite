use crate::oci_common::OciError;
use betterlite_prompts::arguments::json::Json;
use serde::de::DeserializeOwned;
pub(crate) fn parse<T>(value: &Json, status: u16, prefix: &str) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    // serializing to text keeps key order even without serde_json's preserve_order feature
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    serde_json::from_slice(&bytes).map_err(|source| OciError {
        status,
        message: format!("{prefix}: {source}"),
    })
}
