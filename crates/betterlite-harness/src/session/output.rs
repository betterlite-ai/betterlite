use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::payloads::last_json_object;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("no JSON object found in the final answer")]
    NoJsonObject,
    #[error("{0}")]
    Invalid(#[source] serde_json::Error),
}

pub fn parse_output<T>(output_json: Option<&str>, text: &str) -> Result<T, OutputError>
where
    T: DeserializeOwned,
{
    let raw = output_json
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| last_json_object(text))
        .ok_or(OutputError::NoJsonObject)?;
    serde_json::from_str(&raw).map_err(OutputError::Invalid)
}
