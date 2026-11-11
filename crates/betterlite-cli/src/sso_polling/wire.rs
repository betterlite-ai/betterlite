use serde::de::DeserializeOwned;
use serde_json::Deserializer;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WireError {
    #[error(transparent)]
    Shape(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("the body has bytes after the JSON document: {0}")]
    Trailing(#[from] serde_json::Error),
}

pub fn decode<T>(bytes: &[u8]) -> Result<T, WireError>
where
    T: DeserializeOwned,
{
    let mut decoder = Deserializer::from_slice(bytes);
    let value: T = serde_path_to_error::deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}
