use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::models::validate::{RecordError, Validate};

mod tests;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the record is not JSON of the expected shape")]
    Json(#[source] serde_json::Error),
    #[error("the record breaks a field constraint")]
    Invalid(#[source] RecordError),
}

pub fn decode<T>(bytes: &[u8]) -> Result<T, DecodeError>
where
    T: DeserializeOwned + Validate,
{
    let record: T = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    record.validate().map_err(DecodeError::Invalid)?;
    Ok(record)
}
