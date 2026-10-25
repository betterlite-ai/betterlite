use garde::{Report, Validate};
use serde::de::DeserializeOwned;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the record is not JSON of the expected shape")]
    Json(#[source] serde_json::Error),
    #[error("the record breaks a field constraint")]
    Invalid(#[source] Report),
}

pub fn decode<T>(bytes: &[u8]) -> Result<T, DecodeError>
where
    T: DeserializeOwned + Validate<Context = ()>,
{
    let record: T = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    record.validate().map_err(DecodeError::Invalid)?;
    Ok(record)
}
