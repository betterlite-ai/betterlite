use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("the storage response is not a valid row list")]
pub struct RowsError(#[source] pub serde_json::Error);

pub fn decode_rows<T>(bytes: &[u8]) -> Result<Vec<T>, RowsError>
where
    T: DeserializeOwned,
{
    let decoded = serde_json::from_slice(bytes);
    decoded.map_err(RowsError)
}

mod tests;
